#!/usr/bin/env node
import { mkdir, readFile, writeFile } from "node:fs/promises";
import { dirname, extname } from "node:path";
import {
  BRIDGE_REQUIRED_TYPES,
  buildSchemaContext,
  ConnectionResolveError,
  createBackend,
  DatabaseStatsError,
  DIRECT_QUERY_TYPES,
  evaluateRedisCommandSafety,
  evaluateSqlSafety,
  fetchDatabaseStats,
  fetchDatabaseReport,
  buildBatchReportDir,
  buildBatchReportSavePath,
  buildReportSavePath,
  reportTimestamp,
  applyProxyProfileOverride,
  findProxyProfile,
  findProxyProfilesByName,
  formatSchemaContext,
  getDbxDiagnostics,
  hasInlineProxyParams,
  hasProxyProfileRef,
  isMainModule,
  isProxyTunnelProfile,
  loadTunnelProfiles,
  notifyReload,
  postBridge,
  proxyProfileSummary,
  cliConnectionLogOptions,
  pushConnectionLog,
  resolveConnectionRef,
  runWithConnectionLog,
  stderrStreamSink,
  type ConnectionLogOptions,
  resolveConnectionsByIndexRef,
  DEFAULT_PARALLEL_CONCURRENCY,
  MAX_LIST_INDEX_RANGE_WARN_SIZE,
  type Backend,
  type ConnectionConfig,
  type ProxyTunnelConfig,
  supportsHashLineComments,
} from "@dbx-app/node-core";
import { connectionSummary, csvTable, errorPayload, formatCell, formatErrorMessage, mdTable } from "./cli-format.js";

const FILE_CAPABLE_CONNECTION_TYPES = new Set(["sqlite", "duckdb", "access", "h2"]);
const CONNECTION_BATCH_SEPARATOR = "\n---\n\n";

type BatchItemResult<T> =
  | { ok: true; value: T; index: number }
  | { ok: false; error: Error; index: number };

function normalizeBatchError(error: unknown): Error {
  if (error instanceof Error) return error;
  return new Error(String(error));
}

function batchErrorCode(error: Error): string {
  if (error instanceof CliError) return error.code;
  return "ERROR";
}

function isBatchSkipped(error: Error): boolean {
  return error instanceof CliError && error.code === "SKIPPED_UNSUPPORTED";
}

function formatBatchErrorBody(config: ConnectionConfig, error: Error): string {
  if (isBatchSkipped(error)) {
    return `**Skipped** (${config.name}): ${error.message}`;
  }
  const code = error instanceof CliError ? `[${error.code}] ` : "";
  return `**Error** (${config.name}): ${code}${error.message}`;
}

function batchExitCode(results: BatchItemResult<unknown>[]): number {
  return results.some((r) => !r.ok && !isBatchSkipped(r.error)) ? 1 : 0;
}

function formatBatchSummary(results: BatchItemResult<unknown>[], configs: ConnectionConfig[]): string {
  const skipped = results.filter((r): r is Extract<BatchItemResult<unknown>, { ok: false }> => !r.ok && isBatchSkipped(r.error));
  const failures = results.filter((r): r is Extract<BatchItemResult<unknown>, { ok: false }> => !r.ok && !isBatchSkipped(r.error));
  if (failures.length === 0 && skipped.length === 0) return "";
  const successes = results.filter((r) => r.ok).length;
  const parts = [`\n---\n\nBatch: ${successes}/${results.length} succeeded`];
  if (skipped.length > 0) {
    parts.push(
      `Skipped (unsupported):`,
      ...skipped.map((r) => `- #${r.index + 1} ${configs[r.index]!.name}: ${r.error.message}`),
    );
  }
  if (failures.length > 0) {
    parts.push(
      `Failures:`,
      ...failures.map((r) => `- #${r.index + 1} ${configs[r.index]!.name}: ${r.error.message}`),
    );
  }
  return `${parts.join("\n")}\n`;
}

function mapStatsReportError(error: unknown, skipUnsupported: boolean): never {
  if (error instanceof DatabaseStatsError) {
    if (error.code === "UNSUPPORTED_DB_TYPE" && skipUnsupported) {
      throw new CliError("SKIPPED_UNSUPPORTED", error.message);
    }
    throw new CliError(error.code, error.message);
  }
  throw error;
}

function finishBatchOutput(
  stdoutBody: string,
  results: BatchItemResult<unknown>[],
  configs: ConnectionConfig[],
): CliRunResult {
  const summary = configs.length > 1 ? formatBatchSummary(results, configs) : "";
  return { exitCode: batchExitCode(results), stdout: `${stdoutBody}${summary}`, stderr: "" };
}
const DEFAULT_PORTS: Record<string, number> = {
  kwdb: 26257,
  rqlite: 4001,
  "cloudflare-d1": 443,
  tdengine: 6041,
  oscar: 2003,
  iotdb: 6667,
  xugu: 5138,
};

class CliError extends Error {
  readonly code: string;

  constructor(code: string, message: string) {
    super(message);
    this.code = code;
  }
}

export interface CliRunResult {
  exitCode: number;
  stdout: string;
  stderr: string;
}

export interface CliRunOptions {
  env?: NodeJS.ProcessEnv;
  backend?: Backend;
  backendFactory?: (env: NodeJS.ProcessEnv) => Promise<Backend>;
  diagnostics?: typeof getDbxDiagnostics;
}

interface ParsedFlags {
  args: string[];
  json: boolean;
  format: "table" | "json" | "csv";
  allowWrites: boolean;
  allowDangerous: boolean;
  help: boolean;
  version: boolean;
  schema?: string;
  database?: string;
  tables?: string[];
  maxTables?: number;
  maxRows?: number;
  timeoutMs?: number;
  file?: string;
  name?: string;
  dbType?: string;
  host?: string;
  port?: number;
  username?: string;
  password?: string;
  ssl?: boolean;
  driverProfile?: string;
  proxy?: boolean;
  proxyType?: "socks5" | "http";
  proxyHost?: string;
  proxyPort?: number;
  proxyUsername?: string;
  proxyPassword?: string;
  proxyProfileId?: string;
  proxyProfileName?: string;
  quiet: boolean;
  verbose: boolean;
  parallel?: number;
  /** Default true for stats/report: unsupported db types are skipped (not failed). */
  skipUnsupported: boolean;
  noSave: boolean;
  output?: string;
}

export async function runCli(argv: string[], options: CliRunOptions = {}): Promise<CliRunResult> {
  const env = options.env ?? process.env;
  let ownedBackend: Backend | undefined;
  const flags = parseFlags(argv);
  const popConnectionLog = pushConnectionLog(
    cliConnectionLogOptions({
      quiet: flags.quiet || parseBooleanEnv(env.DBX_QUIET),
      verbose: flags.verbose || parseBooleanEnv(env.DBX_VERBOSE),
    }),
  );
  const succeed = (stdout: string) => ok(stdout);
  const succeedJson = (payload: unknown) => okJson(payload);
  const failed = (code: string, message: string, json = flags.json) => fail(code, message, json);

  try {
    const args = flags.args;

    if (flags.version) {
      return succeed(`${await packageVersion()}\n`);
    }
    if (args.length === 0 || flags.help || args[0] === "help") {
      return succeed(`${usage()}\n`);
    }

    const backendFactory = options.backendFactory ?? createBackend;
    const backend = options.backend ?? (ownedBackend = await backendFactory(env));

    if (args[0] === "doctor") {
      ensureArgCount(args, 1, "dbx doctor");
      const diagnostics = await (options.diagnostics ?? getDbxDiagnostics)();
      if (flags.format === "json") return succeedJson(diagnostics);
      if (flags.format === "csv") {
        return succeed(
          csvTable(["check", "value"], [
            { check: "appDataDir", value: diagnostics.appDataDir },
            { check: "dbPath", value: diagnostics.dbPath },
            { check: "dbPathExists", value: diagnostics.dbPathExists },
            { check: "connectionsTableExists", value: diagnostics.connectionsTableExists },
            { check: "connectionRowCount", value: diagnostics.connectionRowCount },
            { check: "loadConnectionsOk", value: diagnostics.loadConnectionsOk },
            { check: "loadedConnectionCount", value: diagnostics.loadedConnectionCount },
            { check: "loadConnectionsError", value: diagnostics.loadConnectionsError ?? "" },
            { check: "loadConnectionsHint", value: diagnostics.loadConnectionsHint ?? "" },
            { check: "bridgePortFile", value: diagnostics.bridgePortFile },
            { check: "bridgePortFileExists", value: diagnostics.bridgePortFileExists },
            { check: "bridgeUrl", value: diagnostics.bridgeUrl ?? "" },
          ]),
        );
      }
      return succeed(formatDoctor(diagnostics));
    }

    if (args[0] === "capabilities") {
      ensureArgCount(args, 1, "dbx capabilities");
      const payload = {
        directQueryTypes: [...DIRECT_QUERY_TYPES],
        bridgeRequiredTypes: [...BRIDGE_REQUIRED_TYPES],
      };
      if (flags.format === "json") return succeedJson(payload);
      if (flags.format === "csv") {
        return succeed(
          csvTable(
            ["mode", "type"],
            [
              ...payload.directQueryTypes.map((type) => ({ mode: "direct", type })),
              ...payload.bridgeRequiredTypes.map((type) => ({ mode: "bridge", type })),
            ],
          ),
        );
      }
      return succeed(
        `${mdTable(
          ["Mode", "Types"],
          [
            ["Direct", payload.directQueryTypes.join(", ")],
            ["Requires DBX Desktop", payload.bridgeRequiredTypes.join(", ")],
          ],
        )}\n`,
      );
    }

    if (args[0] === "connections" && args[1] === "list") {
      ensureArgCount(args, 2, "dbx connections list");
      const connections = await backend.loadConnections();
      const summaries = connections.map((connection, index) => ({
        index: index + 1,
        ...connectionSummary(connection),
      }));
      if (flags.format === "json") return succeedJson({ connections: summaries });
      if (flags.format === "csv") {
        return succeed(csvTable(["index", "id", "name", "type", "host", "port", "database"], summaries));
      }
      return succeed(
        `${mdTable(
          ["#", "ID", "Name", "Type", "Host", "Port", "Database"],
          summaries.map((c) => [String(c.index), c.id, c.name, c.type, c.host, String(c.port), c.database ?? ""]),
        )}\n`,
      );
    }

    if (args[0] === "connections" && args[1] === "add") {
      ensureArgCount(args, 2, "dbx connections add");
      const name = required(flags.name, "--name is required.");
      const dbType = required(flags.dbType, "--type is required.");
      const host = required(flags.host, "--host is required.");
      const existing = await backend.findConnection(name);
      if (existing) throw new CliError("CONNECTION_EXISTS", `Connection "${name}" already exists.`);

      const resolvedPort = flags.port ?? DEFAULT_PORTS[dbType] ?? (FILE_CAPABLE_CONNECTION_TYPES.has(dbType) ? 0 : undefined);
      if (resolvedPort === undefined) {
        throw new CliError("INVALID_ARGUMENT", "Port is required for this database type (use --port).");
      }

      const profileRef = hasProxyProfileRef({
        proxy_profile_id: flags.proxyProfileId,
        proxy_profile_name: flags.proxyProfileName,
      });
      const inlineProxy = hasInlineProxyParams({
        proxy_enabled: flags.proxy,
        proxy_host: flags.proxyHost,
        proxy_port: flags.proxyPort,
        proxy_username: flags.proxyUsername,
        proxy_password: flags.proxyPassword,
      });
      if (profileRef && inlineProxy) {
        throw new CliError(
          "PROXY_CONFLICT",
          "Cannot mix saved proxy reference (--proxy-profile-id/--proxy-profile-name) with inline proxy settings (--proxy, --proxy-host, etc.). Use one mode only.",
        );
      }
      if (profileRef && flags.proxyProfileId?.trim() && flags.proxyProfileName?.trim()) {
        throw new CliError("PROXY_CONFLICT", "Specify either --proxy-profile-id or --proxy-profile-name, not both.");
      }

      const baseConfig: Omit<ConnectionConfig, "id"> = {
        name,
        db_type: dbType,
        host,
        port: resolvedPort,
        username: flags.username ?? "",
        password: flags.password ?? "",
        database: flags.database,
        ssl: flags.ssl ?? false,
        driver_profile: flags.driverProfile,
        ssh_enabled: false,
      } as Omit<ConnectionConfig, "id">;

      let savedProxyLabel = "";
      if (profileRef) {
        const profiles = await loadTunnelProfilesForBackend(backend);
        const proxies = profiles.filter((profile) => profile.type === "proxy");
        if (flags.proxyProfileName?.trim() && !flags.proxyProfileId?.trim()) {
          const matches = findProxyProfilesByName(profiles, flags.proxyProfileName);
          if (matches.length > 1) {
            const lines = matches.map((item) => {
              const proxyIdx = proxies.indexOf(item);
              const num = proxyIdx >= 0 ? proxyIdx + 1 : "?";
              return `- #${num} ${item.id}: ${proxyProfileSummary(item as { type: "proxy" } & ProxyTunnelConfig)}`;
            });
            throw new CliError(
              "AMBIGUOUS_PROXY_PROFILE",
              `Multiple proxy profiles named "${flags.proxyProfileName}". Specify --proxy-profile-id:\n${lines.join("\n")}`,
            );
          }
        }
        const profile = findProxyProfile(profiles, {
          proxy_profile_id: flags.proxyProfileId,
          proxy_profile_name: flags.proxyProfileName,
        });
        if (!profile || !isProxyTunnelProfile(profile)) {
          throw new CliError(
            "PROXY_PROFILE_NOT_FOUND",
            "Proxy profile not found. Use `dbx proxies list` to see saved profiles from DBX Settings > Tunnels.",
          );
        }
        savedProxyLabel = profile.name?.trim() || profile.id;
        Object.assign(baseConfig, applyProxyProfileOverride(baseConfig, profile));
      } else if (flags.proxy) {
        if (!flags.proxyHost?.trim()) {
          throw new CliError("INVALID_ARGUMENT", "--proxy-host is required when --proxy is set.");
        }
        Object.assign(baseConfig, {
          proxy_enabled: true,
          proxy_type: flags.proxyType ?? "socks5",
          proxy_host: flags.proxyHost.trim(),
          proxy_port: flags.proxyPort ?? 1080,
          proxy_username: flags.proxyUsername?.trim() || undefined,
          proxy_password: flags.proxyPassword,
        });
      }

      const config = await backend.addConnection(baseConfig);
      await notifyReload().catch(() => {});
      const proxyNote = savedProxyLabel ? ` using saved proxy profile "${savedProxyLabel}"` : "";
      const payload = { id: config.id, name: config.name, proxy_profile: savedProxyLabel || undefined };
      if (flags.format === "json") return succeedJson({ added: payload });
      return succeed(`Connection "${config.name}" added (id: ${config.id})${proxyNote}.\n`);
    }

    if (args[0] === "connections" && args[1] === "remove") {
      ensureArgCount(args, 3, "dbx connections remove");
      const connectionRef = required(args[2], "Connection name or list index is required.");
      const connections = await backend.loadConnections();
      let target: ConnectionConfig;
      try {
        const byIndex = resolveConnectionsByIndexRef(connections, connectionRef);
        if (byIndex !== undefined) {
          if (byIndex.length > 1) {
            throw new CliError("INVALID_ARGUMENT", "connections remove accepts a single connection, not a range.");
          }
          target = byIndex[0]!;
        } else {
          target = resolveConnectionRef(connections, connectionRef);
        }
      } catch (error) {
        if (error instanceof ConnectionResolveError) throw new CliError(error.code, error.message);
        throw error;
      }
      if (backend.removeConnectionById) {
        const removed = await backend.removeConnectionById(target.id);
        if (!removed) throw new CliError("CONNECTION_NOT_FOUND", `Connection with id "${target.id}" not found.`);
      } else {
        const removed = await backend.removeConnection(target.name);
        if (!removed) throw new CliError("CONNECTION_NOT_FOUND", `Connection "${target.name}" could not be removed.`);
      }
      await notifyReload().catch(() => {});
      const payload = { id: target.id, name: target.name };
      if (flags.format === "json") return succeedJson({ removed: payload });
      return succeed(`Connection "${target.name}" (id: ${target.id}) removed.\n`);
    }

    if (args[0] === "proxies" && args[1] === "list") {
      ensureArgCount(args, 2, "dbx proxies list");
      const profiles = await loadTunnelProfilesForBackend(backend);
      const proxies = profiles.filter((profile) => profile.type === "proxy");
      if (proxies.length === 0) {
        if (flags.format === "json") return succeedJson({ proxies: [] });
        return succeed("No saved proxy profiles found. Create one in DBX Settings > Tunnels.\n");
      }
      const rows = proxies.map((profile, index) => ({
        index: index + 1,
        id: profile.id,
        name: profile.name || "",
        type: profile.proxy_type || "socks5",
        host: profile.host || "",
        port: profile.port || 1080,
        username: profile.username?.trim() || "",
        enabled: profile.enabled === false ? "no" : "yes",
        summary: isProxyTunnelProfile(profile) ? proxyProfileSummary(profile) : profile.name || profile.id,
      }));
      if (flags.format === "json") return succeedJson({ proxies: rows });
      if (flags.format === "csv") {
        return succeed(csvTable(["index", "id", "name", "type", "host", "port", "username", "enabled", "summary"], rows));
      }
      return succeed(
        `${mdTable(
          ["#", "ID", "Name", "Type", "Host", "Port", "Username", "Enabled", "Summary"],
          rows.map((row) => [String(row.index), row.id, row.name, row.type, row.host, String(row.port), row.username, row.enabled, row.summary]),
        )}\n`,
      );
    }

    if (args[0] === "stats") {
      const usesDefaultConnection = !!env.DBX_CONNECTION && args.length === 1;
      ensureArgCount(args, usesDefaultConnection ? 1 : 2, "dbx stats");
      const connectionRef = usesDefaultConnection ? env.DBX_CONNECTION! : required(args[1], "Connection name is required.");
      const configs = await applyOptionalProxyProfileOverride(
        backend,
        await resolveConnectionsForCli(backend, connectionRef),
        flags,
      );
      const batchResults = await runConnectionBatch(configs, flags, async (config) => {
        try {
          return await fetchDatabaseStats(backend, config, {
            database: flags.database,
            schema: flags.schema,
            timeoutMs: flags.timeoutMs,
          });
        } catch (error) {
          mapStatsReportError(error, flags.skipUnsupported);
        }
      });
      const jsonResults = batchResults.map((r, i) => ({
        index: i + 1,
        connection: configs[i].name,
        database: flags.database,
        schema: flags.schema,
        ...(r.ok
          ? { ok: true as const, stats: r.value }
          : {
              ok: false as const,
              error: r.error.message,
              code: batchErrorCode(r.error),
              ...(isBatchSkipped(r.error) ? { skipped: true as const } : {}),
            }),
      }));
      const textParts = batchResults.map((r, i) =>
        `${connectionBatchHeading(configs[i], i + 1, configs.length)}${r.ok ? r.value : formatBatchErrorBody(configs[i], r.error)}`,
      );
      if (flags.format === "json") {
        if (configs.length === 1) {
          if (batchResults[0]!.ok) return succeedJson(jsonResults[0]);
          return finishBatchOutput(`${JSON.stringify(jsonResults[0], null, 2)}\n`, batchResults, configs);
        }
        return finishBatchOutput(`${JSON.stringify({ connections: jsonResults }, null, 2)}\n`, batchResults, configs);
      }
      if (flags.format === "csv") {
        throw new CliError("INVALID_OPTION", "CSV format is not supported for dbx stats.");
      }
      return finishBatchOutput(`${joinBatchOutput(textParts)}\n`, batchResults, configs);
    }

    if (args[0] === "report") {
      const usesDefaultConnection = !!env.DBX_CONNECTION && args.length === 1;
      ensureArgCount(args, usesDefaultConnection ? 1 : 2, "dbx report");
      const connectionRef = usesDefaultConnection ? env.DBX_CONNECTION! : required(args[1], "Connection name is required.");
      const configs = await applyOptionalProxyProfileOverride(
        backend,
        await resolveConnectionsForCli(backend, connectionRef),
        flags,
      );
      const batchResults = await runConnectionBatch(configs, flags, async (config) => {
        try {
          return await fetchDatabaseReport(backend, config, {
            database: flags.database,
            schema: flags.schema,
            timeoutMs: flags.timeoutMs,
          });
        } catch (error) {
          mapStatsReportError(error, flags.skipUnsupported);
        }
      });
      const jsonResults = batchResults.map((r, i) => ({
        index: i + 1,
        connection: configs[i].name,
        database: flags.database,
        schema: flags.schema,
        ...(r.ok
          ? { ok: true as const, report: r.value }
          : {
              ok: false as const,
              error: r.error.message,
              code: batchErrorCode(r.error),
              ...(isBatchSkipped(r.error) ? { skipped: true as const } : {}),
            }),
      }));
      const textParts = batchResults.map((r, i) =>
        `${connectionBatchHeading(configs[i], i + 1, configs.length)}${r.ok ? r.value : formatBatchErrorBody(configs[i], r.error)}`,
      );
      if (flags.format === "json") {
        let result: CliRunResult;
        if (configs.length === 1) {
          if (batchResults[0]!.ok) result = succeedJson(jsonResults[0]);
          else result = finishBatchOutput(`${JSON.stringify(jsonResults[0], null, 2)}\n`, batchResults, configs);
        } else {
          result = finishBatchOutput(`${JSON.stringify({ connections: jsonResults }, null, 2)}\n`, batchResults, configs);
        }
        return appendReportSaveNotice(result, await saveReportFiles(flags, configs, batchResults, jsonResults));
      }
      if (flags.format === "csv") {
        throw new CliError("INVALID_OPTION", "CSV format is not supported for dbx report.");
      }
      const result = finishBatchOutput(`${joinBatchOutput(textParts)}\n`, batchResults, configs);
      return appendReportSaveNotice(result, await saveReportFiles(flags, configs, batchResults, jsonResults));
    }

    if (args[0] === "schema" && args[1] === "list") {
      ensureArgCount(args, 3, "dbx schema list");
      const connectionRef = required(args[2], "Connection name is required.");
      const configs = await resolveConnectionsForCli(backend, connectionRef);
      const batchResults = await runConnectionBatch(configs, flags, async (config) => backend.listTables(config, flags.schema));
      const jsonResults = batchResults.map((r, i) => ({
        index: i + 1,
        connection: configs[i].name,
        schema: flags.schema,
        ...(r.ok
          ? { ok: true as const, tables: r.value.map((table, index) => ({ index: index + 1, ...table })) }
          : { ok: false as const, error: r.error.message, code: batchErrorCode(r.error) }),
      }));
      if (flags.format === "csv" && configs.length > 1) {
        throw new CliError("INVALID_OPTION", "CSV format is not supported for batch schema list.");
      }
      const textParts = batchResults.map((r, i) => {
        const heading = connectionBatchHeading(configs[i], i + 1, configs.length);
        if (!r.ok) return `${heading}${formatBatchErrorBody(configs[i], r.error)}`;
        return `${heading}${mdTable(
          ["#", "Table", "Type"],
          r.value.map((t, idx) => [String(idx + 1), t.name, t.type]),
        )}`;
      });
      if (flags.format === "json") {
        if (configs.length === 1) {
          if (batchResults[0]!.ok) return succeedJson(jsonResults[0]);
          return finishBatchOutput(`${JSON.stringify(jsonResults[0], null, 2)}\n`, batchResults, configs);
        }
        return finishBatchOutput(`${JSON.stringify({ connections: jsonResults }, null, 2)}\n`, batchResults, configs);
      }
      if (flags.format === "csv") {
        const first = batchResults[0];
        if (!first?.ok) return finishBatchOutput("", batchResults, configs);
        return finishBatchOutput(
          csvTable(["index", "name", "type"], first.value.map((table, index) => ({ index: index + 1, ...table }))),
          batchResults,
          configs,
        );
      }
      return finishBatchOutput(`${joinBatchOutput(textParts)}\n`, batchResults, configs);
    }

    if (args[0] === "schema" && args[1] === "describe") {
      ensureArgCount(args, 4, "dbx schema describe");
      const connectionRef = required(args[2], "Connection name is required.");
      const table = required(args[3], "Table name is required.");
      const configs = await resolveConnectionsForCli(backend, connectionRef);
      const batchResults = await runConnectionBatch(configs, flags, async (config) =>
        backend.describeTable(config, table, flags.schema),
      );
      const jsonResults = batchResults.map((r, i) => ({
        index: i + 1,
        connection: configs[i].name,
        schema: flags.schema,
        table,
        ...(r.ok ? { ok: true as const, columns: r.value } : { ok: false as const, error: r.error.message, code: batchErrorCode(r.error) }),
      }));
      if (flags.format === "csv" && configs.length > 1) {
        throw new CliError("INVALID_OPTION", "CSV format is not supported for batch schema describe.");
      }
      const textParts = batchResults.map((r, i) => {
        const heading = connectionBatchHeading(configs[i], i + 1, configs.length);
        if (!r.ok) return `${heading}${formatBatchErrorBody(configs[i], r.error)}`;
        return `${heading}${mdTable(
          ["Column", "Type", "Nullable", "Default", "Comment"],
          r.value.map((c) => [
            c.is_primary_key ? `${c.name} (PK)` : c.name,
            c.data_type,
            c.is_nullable ? "YES" : "NO",
            c.column_default ?? "",
            c.comment ?? "",
          ]),
        )}`;
      });
      if (flags.format === "json") {
        if (configs.length === 1) {
          if (batchResults[0]!.ok) return succeedJson(jsonResults[0]);
          return finishBatchOutput(`${JSON.stringify(jsonResults[0], null, 2)}\n`, batchResults, configs);
        }
        return finishBatchOutput(`${JSON.stringify({ connections: jsonResults }, null, 2)}\n`, batchResults, configs);
      }
      if (flags.format === "csv") {
        const first = batchResults[0];
        if (!first?.ok) return finishBatchOutput("", batchResults, configs);
        return finishBatchOutput(
          csvTable(["name", "data_type", "is_nullable", "is_primary_key", "column_default", "comment"], first.value),
          batchResults,
          configs,
        );
      }
      return finishBatchOutput(`${joinBatchOutput(textParts)}\n`, batchResults, configs);
    }

    if (args[0] === "query") {
      const usesDefaultConnection = !!env.DBX_CONNECTION && args.length === (flags.file ? 1 : 2);
      ensureArgCount(args, usesDefaultConnection ? (flags.file ? 1 : 2) : flags.file ? 2 : 3, "dbx query");
      const connectionRef = usesDefaultConnection ? env.DBX_CONNECTION! : required(args[1], "Connection name is required.");
      if (flags.file && args[2]) {
        throw new CliError("INVALID_ARGUMENT", "Provide SQL either inline or with --file, not both.");
      }
      const sqlArg = usesDefaultConnection ? args[1] : args[2];
      const sql = flags.file ? await readFile(flags.file, "utf-8") : required(sqlArg, "SQL string or --file is required.");
      const configs = await applyOptionalProxyProfileOverride(
        backend,
        await resolveConnectionsForCli(backend, connectionRef),
        flags,
      );
      const envSafety = sqlSafetyFromCliEnv(env);
      if (flags.allowDangerous && !flags.allowWrites && !envSafety.allowWrites) {
        throw new CliError("INVALID_OPTION", "--allow-dangerous-sql requires --allow-writes.");
      }
      const safetyOptions = {
        allowWrites: flags.allowWrites || envSafety.allowWrites,
        allowDangerous: flags.allowDangerous || envSafety.allowDangerous,
        hashLineComments: configs.some((config) => supportsHashLineComments(config.db_type)),
      };
      const safety = evaluateSqlSafety(sql, safetyOptions);
      if (!safety.allowed) return failed("SQL_BLOCKED", safety.reason ?? "SQL blocked.", flags.json);
      const batchResults = await runConnectionBatch(configs, flags, async (config) =>
        backend.executeQuery(config, sql, { maxRows: flags.maxRows, timeoutMs: flags.timeoutMs }),
      );
      if (flags.format === "csv" && configs.length > 1) {
        throw new CliError("INVALID_OPTION", "CSV format is not supported for batch query.");
      }
      const jsonResults = batchResults.map((r, i) => ({
        index: i + 1,
        connection: configs[i].name,
        ...(r.ok
          ? { ok: true as const, columns: r.value.columns, rows: r.value.rows, row_count: r.value.row_count }
          : { ok: false as const, error: r.error.message, code: batchErrorCode(r.error) }),
      }));
      const textParts = batchResults.map((r, i) => {
        const heading = connectionBatchHeading(configs[i], i + 1, configs.length);
        if (!r.ok) return `${heading}${formatBatchErrorBody(configs[i], r.error)}`;
        if (r.value.columns.length === 0) {
          return `${heading}Query executed. ${r.value.row_count} row(s) affected.`;
        }
        return `${heading}${mdTable(
          r.value.columns,
          r.value.rows.map((row) => r.value.columns.map((column) => formatCell(row[column]))),
        )}\n\n${r.value.row_count} row(s)`;
      });
      if (flags.format === "json") {
        if (configs.length === 1) {
          if (batchResults[0]!.ok) return succeedJson(jsonResults[0]);
          return finishBatchOutput(`${JSON.stringify(jsonResults[0], null, 2)}\n`, batchResults, configs);
        }
        return finishBatchOutput(`${JSON.stringify({ connections: jsonResults }, null, 2)}\n`, batchResults, configs);
      }
      if (flags.format === "csv") {
        const first = batchResults[0];
        if (!first?.ok) return finishBatchOutput("", batchResults, configs);
        return finishBatchOutput(csvTable(first.value.columns, first.value.rows), batchResults, configs);
      }
      return finishBatchOutput(`${joinBatchOutput(textParts)}\n`, batchResults, configs);
    }

    if (args[0] === "context") {
      const usesDefaultConnection = !!env.DBX_CONNECTION && args.length === 1;
      ensureArgCount(args, usesDefaultConnection ? 1 : 2, "dbx context");
      const connectionRef = usesDefaultConnection ? env.DBX_CONNECTION! : required(args[1], "Connection name is required.");
      const configs = await resolveConnectionsForCli(backend, connectionRef);
      const batchResults = await runConnectionBatch(configs, flags, async (config) =>
        buildSchemaContext(backend, config, {
          schema: flags.schema,
          tables: flags.tables,
          maxTables: flags.maxTables,
        }),
      );
      const jsonResults = batchResults.map((r, i) =>
        r.ok ? { index: i + 1, ok: true as const, ...r.value } : { index: i + 1, ok: false as const, error: r.error.message, code: batchErrorCode(r.error) },
      );
      const textParts = batchResults.map((r, i) =>
        `${connectionBatchHeading(configs[i], i + 1, configs.length)}${r.ok ? formatSchemaContext(r.value) : formatBatchErrorBody(configs[i], r.error)}`,
      );
      if (flags.format === "json") {
        if (configs.length === 1) {
          if (batchResults[0]!.ok) return succeedJson(jsonResults[0]);
          return finishBatchOutput(`${JSON.stringify(jsonResults[0], null, 2)}\n`, batchResults, configs);
        }
        return finishBatchOutput(`${JSON.stringify({ connections: jsonResults }, null, 2)}\n`, batchResults, configs);
      }
      if (flags.format === "csv") throw new CliError("INVALID_OPTION", "CSV format is not supported for dbx context.");
      return finishBatchOutput(`${joinBatchOutput(textParts)}\n`, batchResults, configs);
    }

    if (args[0] === "redis") {
      ensureArgCount(args, 2, "dbx redis");
      let connectionRef: string;
      let command: string;
      if (args.length === 2) {
        connectionRef = required(env.DBX_CONNECTION, "Connection name is required (or set DBX_CONNECTION).");
        command = required(args[1], "Redis command is required.");
      } else if (env.DBX_CONNECTION) {
        try {
          await resolveConnectionsForCli(backend, args[1]!);
          connectionRef = args[1]!;
          command = args.slice(2).join(" ");
        } catch {
          connectionRef = env.DBX_CONNECTION;
          command = args.slice(1).join(" ");
        }
      } else {
        connectionRef = required(args[1], "Connection name is required.");
        command = args.slice(2).join(" ");
      }
      if (!command.trim()) throw new CliError("INVALID_ARGUMENT", "Redis command is required.");
      const configs = await resolveConnectionsForCli(backend, connectionRef);
      const envSafety = sqlSafetyFromCliEnv(env);
      if (flags.allowDangerous && !flags.allowWrites && !envSafety.allowWrites) {
        throw new CliError("INVALID_OPTION", "--allow-dangerous-sql requires --allow-writes.");
      }
      const safety = evaluateRedisCommandSafety(command, {
        allowWrites: flags.allowWrites || envSafety.allowWrites,
        allowDangerous: flags.allowDangerous || envSafety.allowDangerous,
      });
      if (!safety.allowed) return failed("REDIS_COMMAND_BLOCKED", safety.reason ?? "Redis command blocked.", flags.json);
      const redisDb = redisDbFromCliFlags(flags);
      const batchResults = await runConnectionBatch(configs, flags, async (config) => {
        if (config.db_type !== "redis") {
          throw new CliError("INVALID_CONNECTION_TYPE", `Connection "${config.name}" is ${config.db_type}, not Redis.`);
        }
        if (!backend.executeRedisCommand) {
          throw new CliError("UNSUPPORTED_BACKEND", "This DBX backend does not support Redis command execution.");
        }
        return backend.executeRedisCommand(config, redisDb, command, {
          skipSafetyCheck: safety.skipSafetyCheck,
          timeoutMs: flags.timeoutMs,
        });
      });
      const jsonResults = batchResults.map((r, i) => ({
        index: i + 1,
        connection: configs[i]!.name,
        db: redisDb,
        ...(r.ok
          ? { ok: true as const, command: r.value.command, safety: r.value.safety, value: r.value.value }
          : { ok: false as const, error: r.error.message, code: batchErrorCode(r.error) }),
      }));
      const textParts = batchResults.map((r, i) => {
        const heading = connectionBatchHeading(configs[i]!, i + 1, configs.length);
        if (!r.ok) return `${heading}${formatBatchErrorBody(configs[i]!, r.error)}`;
        const valueText = typeof r.value.value === "string" ? r.value.value : JSON.stringify(r.value.value, null, 2);
        return `${heading}Command: ${r.value.command}\nSafety: ${r.value.safety}\n\n${valueText ?? String(r.value.value)}`;
      });
      if (flags.format === "json") {
        if (configs.length === 1) {
          if (batchResults[0]!.ok) return succeedJson(jsonResults[0]);
          return finishBatchOutput(`${JSON.stringify(jsonResults[0], null, 2)}\n`, batchResults, configs);
        }
        return finishBatchOutput(`${JSON.stringify({ connections: jsonResults }, null, 2)}\n`, batchResults, configs);
      }
      if (flags.format === "csv") throw new CliError("INVALID_OPTION", "CSV format is not supported for dbx redis.");
      return finishBatchOutput(`${joinBatchOutput(textParts)}\n`, batchResults, configs);
    }

    if (args[0] === "open") {
      ensureArgCount(args, 3, "dbx open");
      const connectionRef = required(args[1], "Connection name is required.");
      const table = required(args[2], "Table name is required.");
      const configs = await resolveConnectionsForCli(backend, connectionRef);
      const batchResults = await runConnectionBatch(configs, flags, async (config) => {
        const response = await postBridge("/open-table", {
          connection_name: config.name,
          table,
          schema: flags.schema,
          database: flags.database,
        });
        if (!response.ok) {
          throw new CliError("DBX_NOT_RUNNING", response.text || "DBX is not running. Please start DBX first.");
        }
        return response;
      });
      const jsonResults = batchResults.map((r, i) => ({
        index: i + 1,
        connection: configs[i].name,
        table,
        schema: flags.schema,
        database: flags.database,
        ...(r.ok
          ? { ok: true as const, opened: true }
          : { ok: false as const, error: r.error.message, code: batchErrorCode(r.error) }),
      }));
      const textParts = batchResults.map((r, i) =>
        `${connectionBatchHeading(configs[i], i + 1, configs.length)}${r.ok ? `Opened ${table} in DBX` : formatBatchErrorBody(configs[i], r.error)}`,
      );
      if (flags.format === "json") {
        if (configs.length === 1) {
          if (batchResults[0]!.ok) return succeedJson(jsonResults[0]);
          return finishBatchOutput(`${JSON.stringify(jsonResults[0], null, 2)}\n`, batchResults, configs);
        }
        return finishBatchOutput(`${JSON.stringify({ connections: jsonResults }, null, 2)}\n`, batchResults, configs);
      }
      if (flags.format === "csv") throw new CliError("INVALID_OPTION", "CSV format is not supported for dbx open.");
      return finishBatchOutput(`${joinBatchOutput(textParts)}\n`, batchResults, configs);
    }

    return failed("USAGE", usage(), flags.json);
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error);
    const code =
      error instanceof CliError
        ? error.code
        : error instanceof ConnectionResolveError
          ? error.code
        : typeof error === "object" && error !== null && "code" in error && typeof error.code === "string"
          ? error.code
          : "ERROR";
    const wantsJson = argv.includes("--json") || argv.includes("-j");
    return failed(code, message, wantsJson);
  } finally {
    popConnectionLog();
    await ownedBackend?.close?.().catch(() => {});
  }
}

async function loadTunnelProfilesForBackend(backend: Backend) {
  if (backend.loadTunnelProfiles) return backend.loadTunnelProfiles();
  return loadTunnelProfiles();
}

function parseFlags(argv: string[]): ParsedFlags {
  const args: string[] = [];
  const flags: ParsedFlags = {
    args,
    json: false,
    format: "table",
    allowWrites: false,
    allowDangerous: false,
    help: false,
    version: false,
    quiet: false,
    verbose: false,
    skipUnsupported: true,
    noSave: false,
  };

  for (let i = 0; i < argv.length; i++) {
    const arg = argv[i];
    if (arg === "--") {
      args.push(...argv.slice(i + 1));
      break;
    }
    if (arg === "--json" || arg === "-j") {
      flags.json = true;
      flags.format = "json";
    } else if (arg === "--format") flags.format = parseFormat(readOptionValue(argv, ++i, "--format"));
    else if (arg === "--help" || arg === "-h") flags.help = true;
    else if (arg === "--version" || arg === "-V") flags.version = true;
    else if (arg === "--quiet" || arg === "-q") flags.quiet = true;
    else if (arg === "--verbose" || arg === "-v") flags.verbose = true;
    else if (arg === "--parallel" || arg === "-P") {
      const next = argv[i + 1];
      if (next && !next.startsWith("-") && /^\d+$/.test(next)) {
        flags.parallel = parsePositiveInt(next, arg);
        i++;
      } else {
        flags.parallel = DEFAULT_PARALLEL_CONCURRENCY;
      }
    }
    else if (arg === "--schema" || arg === "-s") flags.schema = readOptionValue(argv, ++i, "--schema");
    else if (arg === "--database" || arg === "-d") flags.database = readOptionValue(argv, ++i, "--database");
    else if (arg === "--tables") flags.tables = splitCsv(readOptionValue(argv, ++i, "--tables"));
    else if (arg === "--max-tables") flags.maxTables = parsePositiveInt(readOptionValue(argv, ++i, "--max-tables"), "--max-tables");
    else if (arg === "--limit") flags.maxRows = parsePositiveInt(readOptionValue(argv, ++i, "--limit"), "--limit");
    else if (arg === "--timeout" || arg === "-t") flags.timeoutMs = parseDurationMs(readOptionValue(argv, ++i, "--timeout"), "--timeout");
    else if (arg === "--file") flags.file = readOptionValue(argv, ++i, "--file");
    else if (arg === "--allow-writes") flags.allowWrites = true;
    else if (arg === "--allow-dangerous-sql") flags.allowDangerous = true;
    else if (arg === "--name") flags.name = readOptionValue(argv, ++i, "--name");
    else if (arg === "--type") flags.dbType = readOptionValue(argv, ++i, "--type");
    else if (arg === "--host") flags.host = readOptionValue(argv, ++i, "--host");
    else if (arg === "--port") flags.port = parsePositiveInt(readOptionValue(argv, ++i, "--port"), "--port");
    else if (arg === "--username") flags.username = readOptionValue(argv, ++i, "--username");
    else if (arg === "--password") flags.password = readOptionValue(argv, ++i, "--password");
    else if (arg === "--ssl") flags.ssl = true;
    else if (arg === "--driver-profile") flags.driverProfile = readOptionValue(argv, ++i, "--driver-profile");
    else if (arg === "--proxy") flags.proxy = true;
    else if (arg === "--proxy-type") flags.proxyType = parseProxyType(readOptionValue(argv, ++i, "--proxy-type"));
    else if (arg === "--proxy-host" || arg === "-H") flags.proxyHost = readOptionValue(argv, ++i, "--proxy-host");
    else if (arg === "--proxy-port") flags.proxyPort = parsePositiveInt(readOptionValue(argv, ++i, "--proxy-port"), "--proxy-port");
    else if (arg === "--proxy-username") flags.proxyUsername = readOptionValue(argv, ++i, "--proxy-username");
    else if (arg === "--proxy-password") flags.proxyPassword = readOptionValue(argv, ++i, "--proxy-password");
    else if (arg === "--proxy-profile-id") flags.proxyProfileId = readOptionValue(argv, ++i, "--proxy-profile-id");
    else if (arg === "--proxy-profile-name") flags.proxyProfileName = readOptionValue(argv, ++i, "--proxy-profile-name");
    else if (arg === "--skip-unsupported") flags.skipUnsupported = true;
    else if (arg === "--no-skip-unsupported") flags.skipUnsupported = false;
    else if (arg === "--no-save" || arg === "-n") flags.noSave = true;
    else if (arg === "--output" || arg === "-o") flags.output = readOptionValue(argv, ++i, arg);
    else if (arg.startsWith("-")) throw new CliError("UNKNOWN_OPTION", `Unknown option: ${arg}`);
    else args.push(arg);
  }

  return flags;
}

function parseFormat(value: string): "table" | "json" | "csv" {
  if (value === "table" || value === "json" || value === "csv") return value;
  throw new CliError("INVALID_OPTION", "--format must be one of: table, json, csv.");
}

function parseProxyType(value: string): "socks5" | "http" {
  if (value === "socks5" || value === "http") return value;
  throw new CliError("INVALID_OPTION", "--proxy-type must be socks5 or http.");
}

function ensureArgCount(args: string[], count: number, command: string) {
  if (args.length !== count) {
    throw new CliError("INVALID_ARGUMENT", `${command} expects ${count - 1} argument(s); received ${args.length - 1}.`);
  }
}

function readOptionValue(argv: string[], index: number, option: string) {
  const value = argv[index];
  if (!value || value.startsWith("-")) {
    throw new CliError("INVALID_OPTION", `${option} requires a value.`);
  }
  return value;
}

function parsePositiveInt(value: string, option: string) {
  const parsed = Number(value);
  if (!Number.isInteger(parsed) || parsed < 1) {
    throw new CliError("INVALID_OPTION", `${option} must be a positive integer.`);
  }
  return parsed;
}

function parseDurationMs(value: string, option: string) {
  const match = value.match(/^(\d+)(ms|s|m)?$/);
  if (!match) {
    throw new CliError("INVALID_OPTION", `${option} must be a positive duration such as 500ms, 10s, or 1m.`);
  }
  const amount = Number(match[1]);
  if (!Number.isInteger(amount) || amount < 1) {
    throw new CliError("INVALID_OPTION", `${option} must be a positive duration such as 500ms, 10s, or 1m.`);
  }
  const unit = match[2] ?? "ms";
  if (unit === "ms") return amount;
  if (unit === "s") return amount * 1000;
  return amount * 60_000;
}

function parseBooleanEnv(value: string | undefined) {
  if (value === undefined) return false;
  const normalized = value.trim().toLowerCase();
  return normalized === "1" || normalized === "true";
}

function sqlSafetyFromCliEnv(env: NodeJS.ProcessEnv) {
  return {
    allowWrites: parseBooleanEnv(env.DBX_MCP_ALLOW_WRITES),
    allowDangerous: parseBooleanEnv(env.DBX_MCP_ALLOW_DANGEROUS_SQL),
  };
}

function redisDbFromCliFlags(flags: Pick<ParsedFlags, "database">): number {
  if (flags.database === undefined || flags.database.trim() === "") return 0;
  const db = Number(flags.database.trim());
  if (!Number.isInteger(db) || db < 0) {
    throw new CliError("INVALID_OPTION", "--database for redis must be a non-negative integer (logical DB index).");
  }
  return db;
}

function splitCsv(value: string) {
  return (value ?? "")
    .split(",")
    .map((part) => part.trim())
    .filter(Boolean);
}

async function resolveConnectionsForCli(backend: Backend, ref: string): Promise<ConnectionConfig[]> {
  const connections = await backend.loadConnections();
  try {
    const byIndex = resolveConnectionsByIndexRef(connections, ref);
    if (byIndex !== undefined) {
      if (byIndex.length > MAX_LIST_INDEX_RANGE_WARN_SIZE) {
        console.warn(
          `[dbx] Warning: range "${ref}" resolves to ${byIndex.length} connections (>${MAX_LIST_INDEX_RANGE_WARN_SIZE}). Consider smaller batches or lower --parallel concurrency.`,
        );
      }
      return byIndex;
    }
    return [resolveConnectionRef(connections, ref)];
  } catch (error) {
    if (error instanceof ConnectionResolveError) {
      throw new CliError(error.code, error.message);
    }
    throw error;
  }
}

/**
 * One-shot proxy profile override for stats/report/query: replaces existing proxy
 * layers on each resolved connection for this request only (does not persist).
 */
async function applyOptionalProxyProfileOverride(
  backend: Backend,
  configs: ConnectionConfig[],
  flags: Pick<ParsedFlags, "proxyProfileId" | "proxyProfileName">,
): Promise<ConnectionConfig[]> {
  const profileRef = hasProxyProfileRef({
    proxy_profile_id: flags.proxyProfileId,
    proxy_profile_name: flags.proxyProfileName,
  });
  if (!profileRef) return configs;
  if (flags.proxyProfileId?.trim() && flags.proxyProfileName?.trim()) {
    throw new CliError("PROXY_CONFLICT", "Specify either --proxy-profile-id or --proxy-profile-name, not both.");
  }
  const profiles = await loadTunnelProfilesForBackend(backend);
  const proxies = profiles.filter((profile) => profile.type === "proxy");
  if (flags.proxyProfileName?.trim() && !flags.proxyProfileId?.trim()) {
    const matches = findProxyProfilesByName(profiles, flags.proxyProfileName);
    if (matches.length > 1) {
      const lines = matches.map((item) => {
        const proxyIdx = proxies.indexOf(item);
        const num = proxyIdx >= 0 ? proxyIdx + 1 : "?";
        return `- #${num} ${item.id}: ${proxyProfileSummary(item as { type: "proxy" } & ProxyTunnelConfig)}`;
      });
      throw new CliError(
        "AMBIGUOUS_PROXY_PROFILE",
        `Multiple proxy profiles named "${flags.proxyProfileName}". Specify --proxy-profile-id:\n${lines.join("\n")}`,
      );
    }
  }
  const profile = findProxyProfile(profiles, {
    proxy_profile_id: flags.proxyProfileId,
    proxy_profile_name: flags.proxyProfileName,
  });
  if (!profile || !isProxyTunnelProfile(profile)) {
    throw new CliError(
      "PROXY_PROFILE_NOT_FOUND",
      "Proxy profile not found. Use `dbx proxies list` to see saved profiles from DBX Settings > Tunnels.",
    );
  }
  return configs.map((config) => applyProxyProfileOverride(config, profile));
}

function batchConnectionLogOptions(
  base: { quiet: boolean; verbose: boolean },
  listIndex: number,
): ConnectionLogOptions {
  const label = `[#${listIndex}]`;
  return {
    quiet: base.quiet,
    verbose: base.verbose,
    sink: (message) => stderrStreamSink()(`${label} ${message}`),
  };
}

async function runConnectionBatch<T>(
  configs: ConnectionConfig[],
  flags: Pick<ParsedFlags, "parallel" | "quiet" | "verbose">,
  worker: (config: ConnectionConfig, index: number) => Promise<T>,
): Promise<BatchItemResult<T>[]> {
  const runOne = async (i: number): Promise<BatchItemResult<T>> => {
    const config = { ...configs[i]! };
    try {
      const value = await runWithConnectionLog(
        batchConnectionLogOptions({ quiet: flags.quiet, verbose: flags.verbose }, i + 1),
        () => worker(config, i),
      );
      return { ok: true, value, index: i };
    } catch (error) {
      const err = normalizeBatchError(error);
      if (!flags.quiet) {
        stderrStreamSink()(`[#${i + 1}] ${configs[i]!.name}: ${err.message}\n`);
      }
      return { ok: false, error: err, index: i };
    }
  };

  if (configs.length <= 1 || flags.parallel === undefined) {
    const results: BatchItemResult<T>[] = [];
    for (let i = 0; i < configs.length; i++) {
      results.push(await runOne(i));
    }
    return results;
  }

  const concurrency = Math.min(flags.parallel, configs.length);
  const results: BatchItemResult<T>[] = new Array(configs.length);
  let cursor = 0;

  const runSlot = async () => {
    while (true) {
      const i = cursor++;
      if (i >= configs.length) return;
      results[i] = await runOne(i);
    }
  };

  await Promise.all(Array.from({ length: concurrency }, runSlot));
  return results;
}

function connectionBatchHeading(config: ConnectionConfig, index: number, total: number): string {
  if (total <= 1) return "";
  return `## #${index} ${config.name}\n\n`;
}

function joinBatchOutput(parts: string[]): string {
  return parts.join(CONNECTION_BATCH_SEPARATOR);
}

function appendReportSaveNotice(result: CliRunResult, savedPaths: string[]): CliRunResult {
  if (savedPaths.length === 0) return result;
  const notice = savedPaths.map((path) => `[dbx] Report saved: ${path}`).join("\n");
  return { ...result, stderr: `${result.stderr}${notice}\n` };
}

function reportSaveExtension(format: ParsedFlags["format"]): "md" | "json" {
  return format === "json" ? "json" : "md";
}

function looksLikeReportOutputFile(path: string): boolean {
  const ext = extname(path).toLowerCase();
  return ext === ".md" || ext === ".json";
}

async function saveReportFiles(
  flags: ParsedFlags,
  configs: ConnectionConfig[],
  batchResults: BatchItemResult<string>[],
  jsonResults: unknown[],
): Promise<string[]> {
  if (flags.noSave) return [];

  const extension = reportSaveExtension(flags.format);
  const timestamp = reportTimestamp();
  const savedPaths: string[] = [];

  if (configs.length === 1) {
    const result = batchResults[0];
    if (!result?.ok) return [];

    const config = configs[0]!;
    const outputPath = flags.output ?? buildReportSavePath({
      connectionName: config.name,
      database: flags.database ?? config.database,
      schema: flags.schema,
      extension,
      timestamp,
    });
    const content =
      flags.format === "json"
        ? `${JSON.stringify(jsonResults[0], null, 2)}\n`
        : result.value.endsWith("\n")
          ? result.value
          : `${result.value}\n`;

    await mkdir(dirname(outputPath), { recursive: true });
    await writeFile(outputPath, content, "utf-8");
    savedPaths.push(outputPath);
    return savedPaths;
  }

  if (flags.output && looksLikeReportOutputFile(flags.output)) {
    throw new CliError(
      "INVALID_OPTION",
      "Batch dbx report requires --output to be a directory (one file per connection). Omit --output to use the default batch folder.",
    );
  }

  const batchDir = flags.output ?? buildBatchReportDir(timestamp);
  await mkdir(batchDir, { recursive: true });

  for (let i = 0; i < configs.length; i++) {
    const result = batchResults[i];
    if (!result?.ok) continue;

    const config = configs[i]!;
    const outputPath = buildBatchReportSavePath(batchDir, {
      connectionName: config.name,
      database: flags.database ?? config.database,
      schema: flags.schema,
      extension,
    });
    const content =
      flags.format === "json"
        ? `${JSON.stringify(jsonResults[i], null, 2)}\n`
        : result.value.endsWith("\n")
          ? result.value
          : `${result.value}\n`;

    await writeFile(outputPath, content, "utf-8");
    savedPaths.push(outputPath);
  }

  return savedPaths;
}

async function findConnectionOrThrow(backend: Backend, ref: string) {
  const connections = await backend.loadConnections();
  try {
    return resolveConnectionRef(connections, ref);
  } catch (error) {
    if (error instanceof ConnectionResolveError) {
      throw new CliError(error.code, error.message);
    }
    throw error;
  }
}

function required(value: string | undefined, message: string) {
  if (!value) throw new Error(message);
  return value;
}

function ok(stdout: string, stderr = ""): CliRunResult {
  return { exitCode: 0, stdout, stderr };
}

function okJson(payload: unknown, stderr = ""): CliRunResult {
  return ok(`${JSON.stringify(payload, null, 2)}\n`, stderr);
}

function fail(code: string, message: string, json: boolean, stderr = ""): CliRunResult {
  const text = json ? `${JSON.stringify(errorPayload(code, message), null, 2)}\n` : `${formatErrorMessage(code, message)}\n`;
  return { exitCode: 1, stdout: "", stderr: `${stderr}${text}` };
}

function usage() {
  return [
    "Usage:",
    "  dbx doctor [-j, --json]",
    "  dbx capabilities [-j, --json]",
    "  dbx connections list [-j, --json]",
    "  dbx connections add --name <name> --type <db_type> --host <host> [--port n] [--username u] [--password p] [-d, --database db] [--ssl] [--driver-profile x]",
    "      [--proxy] [--proxy-type socks5|http] [-H, --proxy-host h] [--proxy-port n] [--proxy-username u] [--proxy-password p]",
    "      [--proxy-profile-id id|# | --proxy-profile-name name|#] [-j, --json]",
    "  dbx connections remove <connection|#> [-j, --json]",
    "  dbx proxies list [-j, --json]",
    "  dbx stats <connection|#|range> [-s, --schema name] [-d, --database name] [-t, --timeout 60s] [-P, --parallel [n]] [--skip-unsupported|--no-skip-unsupported] [-q, --quiet] [-v, --verbose] [-j, --json]",
    "  dbx report <connection|#|range> [-s, --schema name] [-d, --database name] [-t, --timeout 60s] [-P, --parallel [n]] [--skip-unsupported|--no-skip-unsupported] [-q, --quiet] [-v, --verbose] [-j, --json] [-n, --no-save] [-o, --output path]",
    "  dbx schema list <connection|#|range> [-s, --schema name] [-P, --parallel [n]] [-q, --quiet] [-v, --verbose] [-j, --json]",
    "  dbx schema describe <connection|#|range> <table> [-s, --schema name] [-P, --parallel [n]] [-q, --quiet] [-v, --verbose] [-j, --json]",
    "  dbx query <connection|#|range> <sql> [--file path] [--limit n] [-t, --timeout 10s] [--allow-writes] [--allow-dangerous-sql] [-P, --parallel [n]] [-q, --quiet] [-v, --verbose] [-j, --json]",
    "  dbx redis <connection|#|range> <command...> [-d, --database n] [-t, --timeout 10s] [--allow-writes] [--allow-dangerous-sql] [-P, --parallel [n]] [-q, --quiet] [-v, --verbose] [-j, --json]",
    "  dbx context <connection|#|range> [-s, --schema name] [--tables a,b] [--max-tables n] [-P, --parallel [n]] [-q, --quiet] [-v, --verbose] [-j, --json]",
    "  dbx open <connection|#|range> <table> [-s, --schema name] [-d, --database name] [-P, --parallel [n]] [-j, --json]",
    "",
    "Global short options:",
    "  -j, --json           JSON output",
    "  -q, --quiet          Suppress progress on stderr",
    "  -v, --verbose        Extra progress detail (e.g. SQL text)",
    "  -P, --parallel [n]   Concurrent batch (default concurrency 15; -P 3 limits to 3)",
    "  -d, --database NAME  Target database",
    "  -s, --schema NAME    Target schema",
    "  -t, --timeout DUR    Query timeout (e.g. 500ms, 60s, 1m)",
    "  -H, --proxy-host H   Proxy host (connections add)",
    "  -o, --output PATH    Report output file or batch directory (dbx report)",
    "  -n, --no-save        Skip saving report to file (dbx report)",
    "  --skip-unsupported   stats/report: treat unsupported types as skipped (default)",
    "  --no-skip-unsupported stats/report: treat unsupported types as failures",
    "",
    "Connection range (non-interactive CLI only): 1-15, 1..15, 1:15, #1-#15, 23-50 — any valid index range (no span cap).",
    `Parallel batch: -P or --parallel runs connections concurrently (default ${DEFAULT_PARALLEL_CONCURRENCY}); -P 3 limits to 3 at a time. Omit for sequential.`,
  ].join("\n");
}

function formatDoctor(diagnostics: Awaited<ReturnType<typeof getDbxDiagnostics>>) {
  const rows = [
    ["App data directory", diagnostics.appDataDir],
    ["DBX database", diagnostics.dbPathExists ? `found (${diagnostics.dbPath})` : `missing (${diagnostics.dbPath})`],
    ["Connections table", diagnostics.connectionsTableExists ? `${diagnostics.connectionRowCount} row(s)` : "missing"],
    [
      "Connection loading",
      diagnostics.loadConnectionsOk
        ? `ok (${diagnostics.loadedConnectionCount} loaded)`
        : `failed (${diagnostics.loadConnectionsError ?? "unknown error"})`,
    ],
    ...(diagnostics.loadConnectionsHint ? [["Connection fix", diagnostics.loadConnectionsHint]] : []),
    [
      "Desktop bridge",
      diagnostics.bridgePortFileExists ? `available (${diagnostics.bridgeUrl ?? diagnostics.bridgePortFile})` : "not running",
    ],
    ["Direct query types", diagnostics.directQueryTypes.join(", ")],
    ["Bridge-required types", diagnostics.bridgeRequiredTypes.join(", ")],
  ];
  return `${mdTable(["Check", "Value"], rows)}\n`;
}

async function packageVersion() {
  const packageJson = await readFile(new URL("../package.json", import.meta.url), "utf-8");
  const parsed = JSON.parse(packageJson) as { version?: string };
  return parsed.version ?? "0.0.0";
}

async function main() {
  const result = await runCli(process.argv.slice(2));
  if (result.stdout) process.stdout.write(result.stdout);
  if (result.stderr) process.stderr.write(result.stderr);
  process.exitCode = result.exitCode;
}

if (isMainModule(import.meta.url, process.argv[1])) {
  main().catch((error) => {
    console.error(error instanceof Error ? error.message : String(error));
    process.exitCode = 1;
  });
}
