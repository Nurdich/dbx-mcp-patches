#!/usr/bin/env node
import { randomUUID } from "node:crypto";
import { readFile } from "node:fs/promises";
import {
  BRIDGE_REQUIRED_TYPES,
  buildSchemaContext,
  createBackend,
  DatabaseStatsError,
  DIRECT_QUERY_TYPES,
  evaluateSqlSafety,
  fetchDatabaseStats,
  fetchDatabaseReport,
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
  parseListIndex,
  postBridge,
  proxyProfileReferenceLayer,
  proxyProfileSummary,
  resolveConnectionByIndex,
  type Backend,
  type ConnectionConfig,
  type ProxyTunnelConfig,
} from "@dbx-app/node-core";
import { connectionSummary, csvTable, errorPayload, formatCell, formatErrorMessage, mdTable } from "./cli-format.js";

const FILE_CAPABLE_CONNECTION_TYPES = new Set(["sqlite", "duckdb", "access", "h2"]);
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
}

export async function runCli(argv: string[], options: CliRunOptions = {}): Promise<CliRunResult> {
  const env = options.env ?? process.env;
  let ownedBackend: Backend | undefined;

  try {
    const flags = parseFlags(argv);
    const args = flags.args;

    if (flags.version) {
      return ok(`${await packageVersion()}\n`);
    }
    if (args.length === 0 || flags.help || args[0] === "help") {
      return ok(`${usage()}\n`);
    }

    const backendFactory = options.backendFactory ?? createBackend;
    const backend = options.backend ?? (ownedBackend = await backendFactory(env));

    if (args[0] === "doctor") {
      ensureArgCount(args, 1, "dbx doctor");
      const diagnostics = await (options.diagnostics ?? getDbxDiagnostics)();
      if (flags.format === "json") return okJson(diagnostics);
      if (flags.format === "csv") {
        return ok(
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
      return ok(formatDoctor(diagnostics));
    }

    if (args[0] === "capabilities") {
      ensureArgCount(args, 1, "dbx capabilities");
      const payload = {
        directQueryTypes: [...DIRECT_QUERY_TYPES],
        bridgeRequiredTypes: [...BRIDGE_REQUIRED_TYPES],
      };
      if (flags.format === "json") return okJson(payload);
      if (flags.format === "csv") {
        return ok(
          csvTable(
            ["mode", "type"],
            [
              ...payload.directQueryTypes.map((type) => ({ mode: "direct", type })),
              ...payload.bridgeRequiredTypes.map((type) => ({ mode: "bridge", type })),
            ],
          ),
        );
      }
      return ok(
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
      if (flags.format === "json") return okJson({ connections: summaries });
      if (flags.format === "csv") {
        return ok(csvTable(["index", "id", "name", "type", "host", "port", "database"], summaries));
      }
      return ok(
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
        baseConfig.transport_layers = [proxyProfileReferenceLayer(profile, randomUUID())];
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
      if (flags.format === "json") return okJson({ added: payload });
      return ok(`Connection "${config.name}" added (id: ${config.id})${proxyNote}.\n`);
    }

    if (args[0] === "proxies" && args[1] === "list") {
      ensureArgCount(args, 2, "dbx proxies list");
      const profiles = await loadTunnelProfilesForBackend(backend);
      const proxies = profiles.filter((profile) => profile.type === "proxy");
      if (proxies.length === 0) {
        if (flags.format === "json") return okJson({ proxies: [] });
        return ok("No saved proxy profiles found. Create one in DBX Settings > Tunnels.\n");
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
      if (flags.format === "json") return okJson({ proxies: rows });
      if (flags.format === "csv") {
        return ok(csvTable(["index", "id", "name", "type", "host", "port", "username", "enabled", "summary"], rows));
      }
      return ok(
        `${mdTable(
          ["#", "ID", "Name", "Type", "Host", "Port", "Username", "Enabled", "Summary"],
          rows.map((row) => [String(row.index), row.id, row.name, row.type, row.host, String(row.port), row.username, row.enabled, row.summary]),
        )}\n`,
      );
    }

    if (args[0] === "stats") {
      const usesDefaultConnection = !!env.DBX_CONNECTION && args.length === 1;
      ensureArgCount(args, usesDefaultConnection ? 1 : 2, "dbx stats");
      const connectionName = usesDefaultConnection ? env.DBX_CONNECTION! : required(args[1], "Connection name is required.");
      const config = await findConnectionOrThrow(backend, connectionName);
      try {
        const body = await fetchDatabaseStats(backend, config, {
          database: flags.database,
          schema: flags.schema,
        });
        if (flags.format === "json") {
          return okJson({ connection: connectionName, database: flags.database, schema: flags.schema, stats: body });
        }
        if (flags.format === "csv") {
          throw new CliError("INVALID_OPTION", "CSV format is not supported for dbx stats.");
        }
        return ok(`${body}\n`);
      } catch (error) {
        if (error instanceof DatabaseStatsError) {
          throw new CliError(error.code, error.message);
        }
        throw error;
      }
    }

    if (args[0] === "report") {
      const usesDefaultConnection = !!env.DBX_CONNECTION && args.length === 1;
      ensureArgCount(args, usesDefaultConnection ? 1 : 2, "dbx report");
      const connectionName = usesDefaultConnection ? env.DBX_CONNECTION! : required(args[1], "Connection name is required.");
      const config = await findConnectionOrThrow(backend, connectionName);
      try {
        const body = await fetchDatabaseReport(backend, config, {
          database: flags.database,
          schema: flags.schema,
        });
        if (flags.format === "json") {
          return okJson({ connection: connectionName, database: flags.database, schema: flags.schema, report: body });
        }
        if (flags.format === "csv") {
          throw new CliError("INVALID_OPTION", "CSV format is not supported for dbx report.");
        }
        return ok(`${body}\n`);
      } catch (error) {
        if (error instanceof DatabaseStatsError) {
          throw new CliError(error.code, error.message);
        }
        throw error;
      }
    }

    if (args[0] === "schema" && args[1] === "list") {
      ensureArgCount(args, 3, "dbx schema list");
      const connectionName = required(args[2], "Connection name is required.");
      const config = await findConnectionOrThrow(backend, connectionName);
      const tables = await backend.listTables(config, flags.schema);
      if (flags.format === "json") {
        return okJson({
          connection: connectionName,
          schema: flags.schema,
          tables: tables.map((table, index) => ({ index: index + 1, ...table })),
        });
      }
      if (flags.format === "csv") return ok(csvTable(["index", "name", "type"], tables.map((table, index) => ({ index: index + 1, ...table }))));
      return ok(`${mdTable(["#", "Table", "Type"], tables.map((t, i) => [String(i + 1), t.name, t.type]))}\n`);
    }

    if (args[0] === "schema" && args[1] === "describe") {
      ensureArgCount(args, 4, "dbx schema describe");
      const connectionName = required(args[2], "Connection name is required.");
      const table = required(args[3], "Table name is required.");
      const config = await findConnectionOrThrow(backend, connectionName);
      const columns = await backend.describeTable(config, table, flags.schema);
      if (flags.format === "json") return okJson({ connection: connectionName, schema: flags.schema, table, columns });
      if (flags.format === "csv") {
        return ok(csvTable(["name", "data_type", "is_nullable", "is_primary_key", "column_default", "comment"], columns));
      }
      return ok(
        `${mdTable(
          ["Column", "Type", "Nullable", "Default", "Comment"],
          columns.map((c) => [
            c.is_primary_key ? `${c.name} (PK)` : c.name,
            c.data_type,
            c.is_nullable ? "YES" : "NO",
            c.column_default ?? "",
            c.comment ?? "",
          ]),
        )}\n`,
      );
    }

    if (args[0] === "query") {
      const usesDefaultConnection = !!env.DBX_CONNECTION && args.length === (flags.file ? 1 : 2);
      ensureArgCount(args, usesDefaultConnection ? (flags.file ? 1 : 2) : flags.file ? 2 : 3, "dbx query");
      const connectionName = usesDefaultConnection ? env.DBX_CONNECTION! : required(args[1], "Connection name is required.");
      if (flags.file && args[2]) {
        throw new CliError("INVALID_ARGUMENT", "Provide SQL either inline or with --file, not both.");
      }
      const sqlArg = usesDefaultConnection ? args[1] : args[2];
      const sql = flags.file ? await readFile(flags.file, "utf-8") : required(sqlArg, "SQL string or --file is required.");
      const envSafety = sqlSafetyFromCliEnv(env);
      if (flags.allowDangerous && !flags.allowWrites && !envSafety.allowWrites) {
        throw new CliError("INVALID_OPTION", "--allow-dangerous-sql requires --allow-writes.");
      }
      const safetyOptions = {
        allowWrites: flags.allowWrites || envSafety.allowWrites,
        allowDangerous: flags.allowDangerous || envSafety.allowDangerous,
      };
      const safety = evaluateSqlSafety(sql, safetyOptions);
      if (!safety.allowed) return fail("SQL_BLOCKED", safety.reason ?? "SQL blocked.", flags.json);
      const config = await findConnectionOrThrow(backend, connectionName);
      const result = await backend.executeQuery(config, sql, { maxRows: flags.maxRows, timeoutMs: flags.timeoutMs });
      if (flags.format === "json") {
        return okJson({ connection: connectionName, columns: result.columns, rows: result.rows, row_count: result.row_count });
      }
      if (flags.format === "csv") return ok(csvTable(result.columns, result.rows));
      if (result.columns.length === 0) return ok(`Query executed. ${result.row_count} row(s) affected.\n`);
      return ok(
        `${mdTable(
          result.columns,
          result.rows.map((row) => result.columns.map((column) => formatCell(row[column]))),
        )}\n\n${result.row_count} row(s)\n`,
      );
    }

    if (args[0] === "context") {
      const usesDefaultConnection = !!env.DBX_CONNECTION && args.length === 1;
      ensureArgCount(args, usesDefaultConnection ? 1 : 2, "dbx context");
      const connectionName = usesDefaultConnection ? env.DBX_CONNECTION! : required(args[1], "Connection name is required.");
      const config = await findConnectionOrThrow(backend, connectionName);
      const context = await buildSchemaContext(backend, config, {
        schema: flags.schema,
        tables: flags.tables,
        maxTables: flags.maxTables,
      });
      if (flags.format === "json") return okJson(context);
      if (flags.format === "csv") throw new CliError("INVALID_OPTION", "CSV format is not supported for dbx context.");
      return ok(`${formatSchemaContext(context)}\n`);
    }

    if (args[0] === "open") {
      ensureArgCount(args, 3, "dbx open");
      const connectionName = required(args[1], "Connection name is required.");
      const table = required(args[2], "Table name is required.");
      const response = await postBridge("/open-table", {
        connection_name: connectionName,
        table,
        schema: flags.schema,
        database: flags.database,
      });
      if (!response.ok) {
        return fail("DBX_NOT_RUNNING", response.text || "DBX is not running. Please start DBX first.", flags.json);
      }
      if (flags.format === "json") {
        return okJson({ opened: true, connection: connectionName, table, schema: flags.schema, database: flags.database });
      }
      if (flags.format === "csv") throw new CliError("INVALID_OPTION", "CSV format is not supported for dbx open.");
      return ok(`Opened ${table} in DBX\n`);
    }

    return fail("USAGE", usage(), flags.json);
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error);
    const code =
      error instanceof CliError
        ? error.code
        : typeof error === "object" && error !== null && "code" in error && typeof error.code === "string"
          ? error.code
          : "ERROR";
    const wantsJson = argv.includes("--json");
    return fail(code, message, wantsJson);
  } finally {
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
  };

  for (let i = 0; i < argv.length; i++) {
    const arg = argv[i];
    if (arg === "--") {
      args.push(...argv.slice(i + 1));
      break;
    }
    if (arg === "--json") {
      flags.json = true;
      flags.format = "json";
    } else if (arg === "--format") flags.format = parseFormat(readOptionValue(argv, ++i, "--format"));
    else if (arg === "--help" || arg === "-h") flags.help = true;
    else if (arg === "--version" || arg === "-V") flags.version = true;
    else if (arg === "--schema") flags.schema = readOptionValue(argv, ++i, "--schema");
    else if (arg === "--database") flags.database = readOptionValue(argv, ++i, "--database");
    else if (arg === "--tables") flags.tables = splitCsv(readOptionValue(argv, ++i, "--tables"));
    else if (arg === "--max-tables") flags.maxTables = parsePositiveInt(readOptionValue(argv, ++i, "--max-tables"), "--max-tables");
    else if (arg === "--limit") flags.maxRows = parsePositiveInt(readOptionValue(argv, ++i, "--limit"), "--limit");
    else if (arg === "--timeout") flags.timeoutMs = parseDurationMs(readOptionValue(argv, ++i, "--timeout"), "--timeout");
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
    else if (arg === "--proxy-host") flags.proxyHost = readOptionValue(argv, ++i, "--proxy-host");
    else if (arg === "--proxy-port") flags.proxyPort = parsePositiveInt(readOptionValue(argv, ++i, "--proxy-port"), "--proxy-port");
    else if (arg === "--proxy-username") flags.proxyUsername = readOptionValue(argv, ++i, "--proxy-username");
    else if (arg === "--proxy-password") flags.proxyPassword = readOptionValue(argv, ++i, "--proxy-password");
    else if (arg === "--proxy-profile-id") flags.proxyProfileId = readOptionValue(argv, ++i, "--proxy-profile-id");
    else if (arg === "--proxy-profile-name") flags.proxyProfileName = readOptionValue(argv, ++i, "--proxy-profile-name");
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

function splitCsv(value: string) {
  return (value ?? "")
    .split(",")
    .map((part) => part.trim())
    .filter(Boolean);
}

async function findConnectionOrThrow(backend: Backend, ref: string) {
  const connections = await backend.loadConnections();
  const trimmed = ref.trim();
  if (!trimmed) throw new CliError("CONNECTION_NOT_FOUND", "Connection reference is required.");

  const byId = connections.find((connection) => connection.id === trimmed);
  if (byId) return byId;

  const matching = connections.filter((connection) => connection.name.toLowerCase() === trimmed.toLowerCase());
  if (matching.length > 1) {
    const lines = matching.map((connection) => {
      const idx = connections.indexOf(connection);
      const num = idx >= 0 ? idx + 1 : "?";
      return `- #${num} ${connection.id}: ${connection.db_type} @ ${connection.host}:${connection.port}`;
    });
    throw new CliError(
      "AMBIGUOUS_CONNECTION",
      `Multiple connections found with name "${ref}". Specify connection id or list index (#):\n${lines.join("\n")}`,
    );
  }
  if (matching.length === 1) return matching[0];

  const listIndex = parseListIndex(trimmed);
  if (listIndex !== undefined) {
    const config = resolveConnectionByIndex(connections, listIndex);
    if (!config) {
      const hint = connections.length > 0 ? ` Valid range: 1-${connections.length}.` : "";
      throw new CliError("CONNECTION_NOT_FOUND", `Connection index #${listIndex} not found. Run \`dbx connections list\`.${hint}`);
    }
    return config;
  }

  throw new CliError("CONNECTION_NOT_FOUND", `Connection "${ref}" not found.`);
}

function required(value: string | undefined, message: string) {
  if (!value) throw new Error(message);
  return value;
}

function ok(stdout: string): CliRunResult {
  return { exitCode: 0, stdout, stderr: "" };
}

function okJson(payload: unknown): CliRunResult {
  return ok(`${JSON.stringify(payload, null, 2)}\n`);
}

function fail(code: string, message: string, json: boolean): CliRunResult {
  const text = json ? `${JSON.stringify(errorPayload(code, message), null, 2)}\n` : `${formatErrorMessage(code, message)}\n`;
  return { exitCode: 1, stdout: "", stderr: text };
}

function usage() {
  return [
    "Usage:",
    "  dbx doctor [--json]",
    "  dbx capabilities [--json]",
    "  dbx connections list [--json]",
    "  dbx connections add --name <name> --type <db_type> --host <host> [--port n] [--username u] [--password p] [--database db] [--ssl] [--driver-profile x]",
    "      [--proxy] [--proxy-type socks5|http] [--proxy-host h] [--proxy-port n] [--proxy-username u] [--proxy-password p]",
    "      [--proxy-profile-id id|# | --proxy-profile-name name|#] [--json]",
    "  dbx proxies list [--json]",
    "  dbx stats <connection|#> [--schema name] [--database name] [--json]",
    "  dbx report <connection|#> [--schema name] [--database name] [--json]",
    "  dbx schema list <connection|#> [--schema name] [--json]",
    "  dbx schema describe <connection|#> <table> [--schema name] [--json]",
    "  dbx query <connection|#> <sql> [--file path] [--limit n] [--timeout 10s] [--allow-writes] [--allow-dangerous-sql] [--json]",
    "  dbx context <connection|#> [--schema name] [--tables a,b] [--max-tables n] [--json]",
    "  dbx open <connection|#> <table> [--schema name] [--database name] [--json]",
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
