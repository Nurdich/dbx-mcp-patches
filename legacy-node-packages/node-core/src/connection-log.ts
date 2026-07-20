import { AsyncLocalStorage } from "node:async_hooks";
import type { ConnectionConfig } from "./connections.js";

export interface ConnectionLogOptions {
  quiet?: boolean;
  verbose?: boolean;
  sink?: (message: string) => void;
}

const stack: ConnectionLogOptions[] = [];
const logStorage = new AsyncLocalStorage<ConnectionLogOptions>();

function activeOptions(): ConnectionLogOptions {
  return logStorage.getStore() ?? stack.at(-1) ?? { quiet: false, verbose: false };
}

function ensureStderrLineBuffered(): void {
  const handle = (process.stderr as NodeJS.WriteStream & { _handle?: { setBlocking?: (blocking: boolean) => void } })._handle;
  handle?.setBlocking?.(true);
}

function writeStderrLine(message: string): void {
  ensureStderrLineBuffered();
  process.stderr.write(message);
}

function defaultSink(message: string): void {
  writeStderrLine(message);
}

/** Immediate stderr sink for CLI streaming. Optional collect callback for buffering (tests/MCP). */
export function stderrStreamSink(collect?: (message: string) => void): (message: string) => void {
  return (message: string) => {
    writeStderrLine(message);
    collect?.(message);
  };
}

/** CLI options: each connectionLog() line streams to stderr immediately. */
export function cliConnectionLogOptions(opts: { quiet?: boolean; verbose?: boolean } = {}): ConnectionLogOptions {
  return {
    quiet: opts.quiet ?? false,
    verbose: opts.verbose ?? false,
    sink: stderrStreamSink(),
  };
}

export function pushConnectionLog(options: ConnectionLogOptions = {}): () => void {
  const parent = stack.at(-1);
  stack.push({ quiet: false, verbose: false, ...parent, ...options });
  return () => {
    stack.pop();
  };
}

export async function runWithConnectionLog<T>(options: ConnectionLogOptions, fn: () => T | Promise<T>): Promise<T> {
  const parent = activeOptions();
  const merged: ConnectionLogOptions = { quiet: false, verbose: false, ...parent, ...options };
  return logStorage.run(merged, () => fn());
}

export function connectionLog(message: string, opts?: { verboseOnly?: boolean }): void {
  const options = activeOptions();
  if (options.quiet) return;
  if (opts?.verboseOnly && !options.verbose) return;
  const sink = options.sink ?? defaultSink;
  sink(`[dbx] ${message}\n`);
}

/** Log catalog/query SQL on stderr when verbose mode is active. */
export function logQuerySql(sql: string): void {
  const normalized = sql.replace(/\s+/g, " ").trim();
  connectionLog(`SQL: ${normalized}`, { verboseOnly: true });
}

export function describeConnectionTarget(config: ConnectionConfig): string {
  const database = config.database?.trim();
  const target = `${config.db_type} @ ${config.host}:${config.port}`;
  return database ? `${target}/${database}` : target;
}

export function logResolvedConnection(config: ConnectionConfig, ref: string): void {
  connectionLog(`Resolved connection "${config.name}" (${describeConnectionTarget(config)}) from ref "${ref.trim()}"`);
}

export function logTransportLayers(config: ConnectionConfig): void {
  const layers = config.transport_layers?.filter((layer) => layer.enabled !== false) ?? [];
  for (const layer of layers) {
    if (layer.type === "proxy") {
      if (layer.profile_id?.trim()) {
        connectionLog(`Using saved proxy profile: ${layer.name?.trim() || layer.profile_id}`);
      } else {
        connectionLog(`Connecting via ${layer.proxy_type || "socks5"} proxy ${layer.host}:${layer.port || 1080}`);
      }
    } else if (layer.type === "ssh") {
      const label = layer.name?.trim() || layer.profile_id?.trim() || layer.id;
      connectionLog(`SSH tunnel via ${layer.user}@${layer.host}:${layer.port || 22}${label ? ` (${label})` : ""}`);
    }
  }
}

export function connectionStageError(stage: string, error: unknown): Error {
  const detail = error instanceof Error ? error.message : String(error);
  connectionLog(`${stage} failed: ${detail}`);
  const wrapped = new Error(`${stage}: ${detail}`);
  wrapped.name = error instanceof Error ? error.name : "Error";
  if (error instanceof Error && error.stack) wrapped.stack = error.stack;
  return wrapped;
}

export async function withConnectionStage<T>(stage: string, fn: () => Promise<T>): Promise<T> {
  connectionLog(stage.endsWith("...") ? stage : `${stage}...`);
  try {
    return await fn();
  } catch (error) {
    throw connectionStageError(stage.replace(/\.\.\.$/, ""), error);
  }
}

export function parseConnectionLogBooleanEnv(name: string): boolean {
  const value = process.env[name];
  if (!value) return false;
  const normalized = value.trim().toLowerCase();
  return normalized === "1" || normalized === "true" || normalized === "yes";
}

export function mcpConnectionLogOptions(): ConnectionLogOptions {
  return {
    quiet: parseConnectionLogBooleanEnv("DBX_MCP_QUIET"),
    verbose: parseConnectionLogBooleanEnv("DBX_MCP_VERBOSE"),
  };
}

export interface ConnectionLogCollector {
  progress(): string;
  dispose(): void;
}

export function startConnectionLogCollector(options: ConnectionLogOptions = {}, config?: ConnectionConfig): ConnectionLogCollector {
  const logs: string[] = [];
  const dispose = pushConnectionLog({
    quiet: false,
    verbose: false,
    ...options,
    // MCP: buffer only; prepend in tool response. CLI uses cliConnectionLogOptions() instead.
    sink: (message) => logs.push(message),
  });
  if (config) {
    connectionLog(`Using connection "${config.name}" (${describeConnectionTarget(config)})`);
    logTransportLayers(config);
  }
  return {
    progress: () => logs.join(""),
    dispose,
  };
}

export function formatConnectionProgressSection(progress: string): string {
  const trimmed = progress.trimEnd();
  if (!trimmed) return "";
  return `${trimmed}\n\n---\n\n`;
}

export function prependConnectionProgress(body: string, progress: string): string {
  const section = formatConnectionProgressSection(progress);
  return section ? `${section}${body}` : body;
}
