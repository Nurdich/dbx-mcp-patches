#!/usr/bin/env node
import { McpServer } from "@modelcontextprotocol/sdk/server/mcp.js";
import { StdioServerTransport } from "@modelcontextprotocol/sdk/server/stdio.js";
import { createRequire } from "node:module";
import { z } from "zod";
import {
  buildSchemaContext,
  ConnectionResolveError,
  createBackend,
  evaluateRedisCommandSafety,
  evaluateMongoAggregateSafety,
  evaluateSqlSafety,
  formatCell,
  formatSchemaContext,
  isMainModule,
  mdTable,
  notifyReload,
  parseListIndex,
  parseMongoAggregateCommand,
  assessProductionSql,
  isLikelyMongoMutation,
  isProductionDatabase,
  postBridge,
  logSqlDiagnostic,
  resolveConnectionByIndex,
  resolveConnectionsByIndexRef,
  sqlSafetyFromEnv,
  splitSqlStatements,
  supportsHashLineComments,
  DatabaseStatsError,
  fetchDatabaseStats,
  fetchDatabaseReport,
  mcpConnectionLogOptions,
  prependConnectionProgress,
  startConnectionLogCollector,
  type Backend,
  type ConnectionConfig,
  type QueryResult,
  type RedisCommandResult,
} from "@dbx-app/node-core";
import {
  applyProxyProfileOverride,
  findProxyProfile,
  findProxyProfilesByName,
  hasInlineProxyParams,
  hasProxyProfileRef,
  loadTunnelProfiles,
  proxyProfileSummary,
} from "./tunnel-profiles.js";

const require = createRequire(import.meta.url);
const packageJson = require("../package.json") as { version?: string };
export const DBX_MCP_PACKAGE_VERSION = packageJson.version ?? "0.0.0";

function text(s: string) {
  return { content: [{ type: "text" as const, text: s }] };
}

function toolError(code: string, message: string) {
  return { ...text(`${code}: ${message}`), isError: true };
}

function withDatabase(config: ConnectionConfig, database?: string): ConnectionConfig {
  return database === undefined ? config : { ...config, database };
}

function metadataScope(config: ConnectionConfig, database?: string, schema?: string): { config: ConnectionConfig; schema?: string } {
  if (config.db_type !== "dameng") {
    return { config: withDatabase(config, database), schema };
  }

  // Dameng exposes tables under user-owned schemas rather than separate
  // databases. Accept the legacy database argument as a schema, and default to
  // the login user when neither argument is provided.
  const resolvedSchema = schema?.trim() || database?.trim() || config.username?.trim() || undefined;
  return { config, schema: resolvedSchema };
}

function connectionIdentity(config: ConnectionConfig): string {
  return `${config.name} (${config.id}) [${config.db_type} @ ${config.host}:${config.port}]`;
}

function labeledText(config: ConnectionConfig, body: string): ReturnType<typeof text> {
  return text(`[${connectionIdentity(config)}]\n${body}`);
}

function labeledTextWithProgress(config: ConnectionConfig, body: string, progress: string): ReturnType<typeof text> {
  return labeledText(config, prependConnectionProgress(body, progress));
}

function toolResultWithProgress(result: ReturnType<typeof text>, progress: string): ReturnType<typeof text> {
  const body = result.content[0]?.text ?? "";
  return text(prependConnectionProgress(body, progress));
}

function toolErrorWithProgress(code: string, message: string, progress: string) {
  return { ...text(prependConnectionProgress(`${code}: ${message}`, progress)), isError: true };
}

async function runConnectingTool<T>(
  config: ConnectionConfig,
  run: () => Promise<T>,
): Promise<{ value: T; progress: string } | { progress: string; error: unknown }> {
  const collector = startConnectionLogCollector(mcpConnectionLogOptions(), config);
  try {
    const value = await run();
    return { value, progress: collector.progress() };
  } catch (error) {
    return { progress: collector.progress(), error };
  } finally {
    collector.dispose();
  }
}

function formatQueryToolResult(result: QueryResult, title?: string) {
  const prefix = title ? `${title}\n` : "";
  if (result.columns.length === 0) return text(`${prefix}Query executed. ${result.row_count} row(s) affected.`);
  const rows = result.rows.map((r) => result.columns.map((c) => formatCell(r[c])));
  return text(`${prefix}${mdTable(result.columns, rows)}\n\n${result.row_count} row(s)`);
}

function redisDbFromValue(value?: string): number | undefined {
  const trimmed = value?.trim();
  if (!trimmed) return undefined;
  const db = Number(trimmed);
  return Number.isInteger(db) && db >= 0 ? db : undefined;
}

function defaultRedisDb(config: ConnectionConfig, scope: McpScope, db?: number): number {
  return db ?? redisDbFromValue(scope.database) ?? redisDbFromValue(config.database) ?? 0;
}

function formatRedisCommandValue(value: unknown): string {
  if (typeof value === "string") return value;
  return JSON.stringify(value, null, 2) ?? String(value);
}

function formatRedisCommandToolResult(result: RedisCommandResult) {
  return text(`Command: ${result.command}\nSafety: ${result.safety}\n\n${formatRedisCommandValue(result.value)}`);
}

export const DBX_CONNECTION_TYPE_DESCRIPTION =
  "Database type: postgres, mysql, sqlite, rqlite, cloudflare-d1, redis, duckdb, clickhouse, sqlserver, mongodb, oracle, elasticsearch, etcd, doris, starrocks, manticoresearch, milvus, qdrant, weaviate, chromadb, redshift, dameng, kingbase, highgo, vastbase, goldendb, databend, gaussdb, kwdb, yashandb, databricks, saphana, teradata, vertica, firebird, exasol, opengauss, oceanbase-oracle, questdb, gbase, h2, snowflake, trino, prestosql, hive, spark, db2, informix, influxdb, iris, neo4j, cassandra, bigquery, kylin, sundb, oscar, tdengine, iotdb, xugu, zookeeper, jdbc, access, mq";
const FILE_CAPABLE_CONNECTION_TYPES = new Set(["sqlite", "duckdb", "access", "h2"]);

interface McpScope {
  connectionId?: string;
  connectionName?: string;
  database?: string;
}

function scopedValue(value: string | undefined): string | undefined {
  const trimmed = value?.trim();
  return trimmed ? trimmed : undefined;
}

function mcpScopeFromEnv(): McpScope {
  return {
    connectionId: scopedValue(process.env.DBX_MCP_SCOPE_CONNECTION_ID),
    connectionName: scopedValue(process.env.DBX_MCP_SCOPE_CONNECTION_NAME),
    database: scopedValue(process.env.DBX_MCP_SCOPE_DATABASE),
  };
}

function scopeEnabled(scope: McpScope): boolean {
  return !!(scope.connectionId || scope.connectionName);
}

function connectionMatchesScope(config: ConnectionConfig, scope: McpScope): boolean {
  return (!!scope.connectionId && config.id === scope.connectionId) || (!!scope.connectionName && config.name === scope.connectionName);
}

async function loadScopedConnections(backend: Backend, scope: McpScope): Promise<ConnectionConfig[]> {
  const connections = await backend.loadConnections();
  if (!scopeEnabled(scope)) return connections;
  return connections.filter((config) => connectionMatchesScope(config, scope));
}

const MCP_BATCH_SEPARATOR = "\n---\n\n";
const CONNECTION_REF_HINT = "Name, list index (#), or range (e.g. 1-15, 1..15) from dbx_list_connections";

function connectionBatchHeading(config: ConnectionConfig, index: number, total: number): string {
  if (total <= 1) return "";
  return `### #${index} ${connectionIdentity(config)}\n\n`;
}

function formatMcpBatchSummary(successes: number, skipped: number, failures: number, total: number): string {
  if (total <= 1 || (skipped === 0 && failures === 0)) return "";
  const parts = [`\n---\n\nBatch: ${successes}/${total} succeeded`];
  if (skipped > 0) parts.push(`Skipped (unsupported): ${skipped}`);
  if (failures > 0) parts.push(`Failures: ${failures}`);
  return `${parts.join("\n")}\n`;
}

/** Resolve one or more connections; supports list index ranges like CLI (`1-15`, `1..15`, `#1-#15`). */
async function resolveConnections(
  backend: Backend,
  scope: McpScope,
  requestedId?: string,
  requestedName?: string,
): Promise<{ configs?: ConnectionConfig[]; error?: ReturnType<typeof toolError> }> {
  const connections = await loadScopedConnections(backend, scope);

  const tryIndexRange = (ref: string): { matched: boolean; configs?: ConnectionConfig[]; error?: ReturnType<typeof toolError> } => {
    try {
      const byIndex = resolveConnectionsByIndexRef(connections, ref);
      if (byIndex === undefined) return { matched: false };
      return { matched: true, configs: byIndex };
    } catch (error) {
      if (error instanceof ConnectionResolveError) {
        return { matched: true, error: toolError(error.code, error.message) };
      }
      throw error;
    }
  };

  const resolveFromListIndex = (index: number, label: string) => {
    const config = resolveConnectionByIndex(connections, index);
    if (!config) {
      const hint = connections.length > 0 ? ` Valid range: 1-${connections.length}.` : "";
      return { error: toolError("CONNECTION_NOT_FOUND", `Connection index #${index} not found (${label}). Use dbx_list_connections to see available connections.${hint}`) };
    }
    return { configs: [config] };
  };

  // connection_id takes priority over connection_name when both are provided.
  if (requestedId?.trim()) {
    const trimmed = requestedId.trim();
    const config = connections.find((c) => c.id === trimmed);
    if (config) return { configs: [config] };
    const ranged = tryIndexRange(trimmed);
    if (ranged.matched) {
      if (ranged.error) return { error: ranged.error };
      return { configs: ranged.configs };
    }
    return { error: toolError("CONNECTION_NOT_FOUND", `Connection with id "${requestedId}" not found.`) };
  }

  if (!scopeEnabled(scope)) {
    if (!requestedName?.trim()) return { error: toolError("CONNECTION_NOT_FOUND", "Connection name is required.") };
    const trimmed = requestedName.trim();
    const matching = connections.filter((c) => c.name.toLowerCase() === trimmed.toLowerCase());
    if (matching.length === 0) {
      const ranged = tryIndexRange(trimmed);
      if (ranged.matched) {
        if (ranged.error) return { error: ranged.error };
        return { configs: ranged.configs };
      }
      return { error: toolError("CONNECTION_NOT_FOUND", `Connection "${requestedName}" not found.`) };
    }
    if (matching.length > 1) {
      const lines = matching.map((c) => {
        const idx = connections.indexOf(c);
        const num = idx >= 0 ? idx + 1 : "?";
        return `- #${num} ${c.id}: ${c.db_type} @ ${c.host}:${c.port}`;
      });
      return {
        error: toolError("AMBIGUOUS_CONNECTION", `Multiple connections found with name "${requestedName}". Please specify connection_id or list index (#):\n${lines.join("\n")}`),
      };
    }
    return { configs: [matching[0]!] };
  }

  const [scopedConfig] = connections;
  if (!scopedConfig) return { error: toolError("CONNECTION_NOT_FOUND", "Scoped DBX connection was not found.") };
  if (requestedName?.trim()) {
    const trimmed = requestedName.trim();
    if (trimmed !== scopedConfig.name && trimmed !== scopedConfig.id) {
      const ranged = tryIndexRange(trimmed);
      if (ranged.matched) {
        if (ranged.error) return { error: ranged.error };
        if (ranged.configs?.length === 1 && ranged.configs[0] === scopedConfig) return { configs: [scopedConfig] };
        if (ranged.configs?.length === 1 && connections.length === 1 && parseListIndex(trimmed) === 1) return { configs: [scopedConfig] };
        return { error: toolError("CONNECTION_OUT_OF_SCOPE", `Connection range "${requestedName}" is outside this DBX AI session scope.`) };
      }
      const listIndex = parseListIndex(trimmed);
      if (listIndex === 1 && connections.length === 1) return { configs: [scopedConfig] };
      if (listIndex !== undefined) return resolveFromListIndex(listIndex, `connection_name "${trimmed}"`);
      return { error: toolError("CONNECTION_OUT_OF_SCOPE", `Connection "${requestedName}" is outside this DBX AI session scope.`) };
    }
  }
  return { configs: [scopedConfig] };
}

async function resolveConnection(
  backend: Backend,
  scope: McpScope,
  requestedId?: string,
  requestedName?: string,
): Promise<{ config?: ConnectionConfig; error?: ReturnType<typeof toolError> }> {
  const { configs, error } = await resolveConnections(backend, scope, requestedId, requestedName);
  if (error) return { error };
  if (!configs?.length) return { error: toolError("CONNECTION_NOT_FOUND", "Connection name is required.") };
  if (configs.length > 1) {
    return {
      error: toolError(
        "CONNECTION_RANGE",
        `Connection range resolves to ${configs.length} connections. This tool accepts a single connection; use a single index/name, or a batch-capable tool (dbx_get_database_stats / dbx_get_database_report / dbx_execute_query / dbx_list_tables / dbx_describe_table / dbx_get_schema_context).`,
      ),
    };
  }
  return { config: configs[0] };
}

/**
 * One-shot proxy profile override: replaces existing proxy layers for this request only.
 */
async function resolveConnectionsWithProxyOverride(
  backend: Backend,
  scope: McpScope,
  isWebMode: boolean,
  args: {
    connection_id?: string;
    connection_name?: string;
    proxy_profile_id?: string;
    proxy_profile_name?: string;
  },
): Promise<{ configs?: ConnectionConfig[]; error?: ReturnType<typeof toolError> }> {
  const { configs, error } = await resolveConnections(backend, scope, args.connection_id, args.connection_name);
  if (error) return { error };
  if (!hasProxyProfileRef(args)) return { configs };

  if (args.proxy_profile_id?.trim() && args.proxy_profile_name?.trim()) {
    return { error: toolError("PROXY_CONFLICT", "Specify either proxy_profile_id or proxy_profile_name, not both.") };
  }

  const profiles = await loadTunnelProfiles(isWebMode);
  const proxies = profiles.filter((profile) => profile.type === "proxy");
  if (args.proxy_profile_name?.trim() && !args.proxy_profile_id?.trim()) {
    const matches = findProxyProfilesByName(profiles, args.proxy_profile_name);
    if (matches.length > 1) {
      const lines = matches.map((item) => {
        const proxyIdx = proxies.indexOf(item);
        const num = proxyIdx >= 0 ? proxyIdx + 1 : "?";
        return `- #${num} ${item.id}: ${proxyProfileSummary(item)}`;
      });
      return {
        error: toolError(
          "AMBIGUOUS_PROXY_PROFILE",
          `Multiple proxy profiles named "${args.proxy_profile_name}". Specify proxy_profile_id or list index (#):\n${lines.join("\n")}`,
        ),
      };
    }
  }
  const profile = findProxyProfile(profiles, {
    proxy_profile_id: args.proxy_profile_id,
    proxy_profile_name: args.proxy_profile_name,
  });
  if (!profile || profile.type !== "proxy") {
    return {
      error: toolError(
        "PROXY_PROFILE_NOT_FOUND",
        "Proxy profile not found. Use dbx_list_proxies to see saved profiles from DBX Settings > Tunnels.",
      ),
    };
  }
  return { configs: configs!.map((config) => applyProxyProfileOverride(config, profile)) };
}

async function resolveConnectionWithProxyOverride(
  backend: Backend,
  scope: McpScope,
  isWebMode: boolean,
  args: {
    connection_id?: string;
    connection_name?: string;
    proxy_profile_id?: string;
    proxy_profile_name?: string;
  },
): Promise<{ config?: ConnectionConfig; error?: ReturnType<typeof toolError> }> {
  const { configs, error } = await resolveConnectionsWithProxyOverride(backend, scope, isWebMode, args);
  if (error) return { error };
  if (!configs?.length) return { error: toolError("CONNECTION_NOT_FOUND", "Connection name is required.") };
  if (configs.length > 1) {
    return {
      error: toolError(
        "CONNECTION_RANGE",
        `Connection range resolves to ${configs.length} connections. This tool accepts a single connection; use a batch-capable tool or a single index/name.`,
      ),
    };
  }
  return { config: configs[0] };
}

export function createDbxMcpServer(backend: Backend, options: { isWebMode?: boolean } = {}): McpServer {
  const isWebMode = options.isWebMode ?? !!process.env.DBX_WEB_URL;
  const scope = mcpScopeFromEnv();
  const scoped = scopeEnabled(scope);
  const server = new McpServer({
    name: "dbx",
    version: DBX_MCP_PACKAGE_VERSION,
  });

  server.tool("dbx_list_connections", "List all database connections configured in DBX", {}, async () => {
    const connections = await loadScopedConnections(backend, scope);
    if (connections.length === 0) return text("No connections configured in DBX.");
    const rows = connections.map((c, i) => [String(i + 1), c.id, c.name, c.db_type, c.host, String(c.port), c.database || ""]);
    return text(mdTable(["#", "ID", "Name", "Type", "Host", "Port", "Database"], rows));
  });

  server.tool("dbx_list_proxies", "List saved proxy tunnel profiles from DBX Settings > Tunnels", {}, async () => {
    const profiles = await loadTunnelProfiles(isWebMode);
    const proxies = profiles.filter((profile) => profile.type === "proxy");
    if (proxies.length === 0) {
      return text("No saved proxy profiles found. Create one in DBX Settings > Tunnels.");
    }
    const rows = proxies.map((profile, i) => [
      String(i + 1),
      profile.id,
      profile.name || "",
      profile.proxy_type || "socks5",
      profile.host || "",
      String(profile.port || 1080),
      profile.username?.trim() || "",
      profile.enabled === false ? "no" : "yes",
      proxyProfileSummary(profile),
    ]);
    return text(mdTable(["#", "ID", "Name", "Type", "Host", "Port", "Username", "Enabled", "Summary"], rows));
  });

  server.tool(
    "dbx_list_tables",
    "List tables and views for a database connection. Accepts connection ranges (e.g. 1-15); runs sequentially.",
    {
      connection_id: z.string().optional().describe(`Unique ID of the DBX connection, or list index/range from dbx_list_connections (e.g. 1, #2, 1-15)`),
      connection_name: z.string().optional().describe(CONNECTION_REF_HINT),
      database: z.string().optional().describe("Database name; for Dameng this is also accepted as a schema alias"),
      schema: z.string().optional().describe("Schema name (default: public for PostgreSQL, login user for Dameng)"),
    },
    async ({ connection_id, connection_name, database, schema }) => {
      const { configs, error } = await resolveConnections(backend, scope, connection_id, connection_name);
      if (error) return error;
      const resolvedConfigs = configs!;
      const parts: string[] = [];
      let failures = 0;
      for (let i = 0; i < resolvedConfigs.length; i++) {
        const resolvedConfig = resolvedConfigs[i]!;
        const scopeValue = metadataScope(resolvedConfig, database ?? scope.database, schema);
        const outcome = await runConnectingTool(resolvedConfig, () => backend.listTables(scopeValue.config, scopeValue.schema));
        const heading = connectionBatchHeading(resolvedConfig, i + 1, resolvedConfigs.length);
        if ("error" in outcome) {
          failures++;
          const msg = outcome.error instanceof Error ? outcome.error.message : String(outcome.error);
          parts.push(prependConnectionProgress(`${heading}**Error** (${resolvedConfig.name}): LIST_TABLES_ERROR: ${msg}`, outcome.progress));
          continue;
        }
        const tables = outcome.value;
        const body =
          tables.length === 0
            ? "No tables found."
            : mdTable(
                ["#", "Table", "Type"],
                tables.map((t, idx) => [String(idx + 1), t.name, t.type]),
              );
        parts.push(
          resolvedConfigs.length === 1
            ? labeledTextWithProgress(resolvedConfig, body, outcome.progress).content[0]!.text
            : prependConnectionProgress(`${heading}${body}`, outcome.progress),
        );
      }
      const summary = formatMcpBatchSummary(resolvedConfigs.length - failures, 0, failures, resolvedConfigs.length);
      const result = text(`${parts.join(MCP_BATCH_SEPARATOR)}${summary}`);
      return failures > 0 && resolvedConfigs.length > 1 ? { ...result, isError: true } : result;
    },
  );

  server.tool(
    "dbx_describe_table",
    "Get column definitions for a table. Accepts connection ranges (e.g. 1-15); runs sequentially.",
    {
      connection_id: z.string().optional().describe(`Unique ID of the DBX connection, or list index/range from dbx_list_connections (e.g. 1, #2, 1-15)`),
      connection_name: z.string().optional().describe(CONNECTION_REF_HINT),
      table: z.string().describe("Table name"),
      database: z.string().optional().describe("Database name; for Dameng this is also accepted as a schema alias"),
      schema: z.string().optional().describe("Schema name (default: public for PostgreSQL, login user for Dameng)"),
    },
    async ({ connection_id, connection_name, table, database, schema }) => {
      const { configs, error } = await resolveConnections(backend, scope, connection_id, connection_name);
      if (error) return error;
      const resolvedConfigs = configs!;
      const parts: string[] = [];
      let failures = 0;
      for (let i = 0; i < resolvedConfigs.length; i++) {
        const resolvedConfig = resolvedConfigs[i]!;
        const scopeValue = metadataScope(resolvedConfig, database ?? scope.database, schema);
        const outcome = await runConnectingTool(resolvedConfig, () => backend.describeTable(scopeValue.config, table, scopeValue.schema));
        const heading = connectionBatchHeading(resolvedConfig, i + 1, resolvedConfigs.length);
        if ("error" in outcome) {
          failures++;
          const msg = outcome.error instanceof Error ? outcome.error.message : String(outcome.error);
          parts.push(prependConnectionProgress(`${heading}**Error** (${resolvedConfig.name}): DESCRIBE_TABLE_ERROR: ${msg}`, outcome.progress));
          continue;
        }
        const columns = outcome.value;
        const body =
          columns.length === 0
            ? "No columns found."
            : mdTable(
                ["Column", "Type", "Nullable", "Default", "Comment"],
                columns.map((c) => [
                  c.is_primary_key ? `${c.name} (PK)` : c.name,
                  c.data_type,
                  c.is_nullable ? "YES" : "NO",
                  c.column_default ?? "",
                  c.comment ?? "",
                ]),
              );
        parts.push(
          resolvedConfigs.length === 1
            ? labeledTextWithProgress(resolvedConfig, body, outcome.progress).content[0]!.text
            : prependConnectionProgress(`${heading}${body}`, outcome.progress),
        );
      }
      const summary = formatMcpBatchSummary(resolvedConfigs.length - failures, 0, failures, resolvedConfigs.length);
      const result = text(`${parts.join(MCP_BATCH_SEPARATOR)}${summary}`);
      return failures > 0 && resolvedConfigs.length > 1 ? { ...result, isError: true } : result;
    },
  );

  server.tool(
    "dbx_get_database_stats",
    "Get database status overview from system catalog views (information_schema, pg_catalog, sqlite_master, etc.): table metadata, size estimates, and row estimates without manual COUNT queries. Accepts connection ranges (e.g. 1-15); runs sequentially (no --parallel).",
    {
      connection_id: z.string().optional().describe(`Unique ID of the DBX connection, or list index/range from dbx_list_connections (e.g. 1, #2, 1-15)`),
      connection_name: z.string().optional().describe(CONNECTION_REF_HINT),
      database: z.string().optional().describe("Database name; for Dameng this is also accepted as a schema alias"),
      schema: z.string().optional().describe("Schema name (default: public for PostgreSQL, dbo for SQL Server, login user for Dameng)"),
      timeout_ms: z.number().int().min(1).optional().describe("Per-connection query timeout in milliseconds (aligned with CLI -t/--timeout)"),
      skip_unsupported: z
        .boolean()
        .optional()
        .describe("When true (default), unsupported db types return SKIPPED_UNSUPPORTED instead of hard error — aligned with CLI --skip-unsupported"),
      proxy_profile_id: z.string().optional().describe("One-shot: replace connection proxy with this saved profile ID or list index (#) for this request only"),
      proxy_profile_name: z.string().optional().describe("One-shot: replace connection proxy with this saved profile name or list index (#) for this request only"),
    },
    async ({ connection_id, connection_name, database, schema, timeout_ms, skip_unsupported, proxy_profile_id, proxy_profile_name }) => {
      const { configs, error } = await resolveConnectionsWithProxyOverride(backend, scope, isWebMode, {
        connection_id,
        connection_name,
        proxy_profile_id,
        proxy_profile_name,
      });
      if (error) return error;
      const resolvedConfigs = configs!;
      const skipUnsupported = skip_unsupported !== false;
      const parts: string[] = [];
      let successes = 0;
      let skipped = 0;
      let failures = 0;
      for (let i = 0; i < resolvedConfigs.length; i++) {
        const resolvedConfig = resolvedConfigs[i]!;
        const outcome = await runConnectingTool(resolvedConfig, () =>
          fetchDatabaseStats(backend, resolvedConfig, {
            database: database ?? scope.database,
            schema,
            redisDb: redisDbFromValue(database) ?? redisDbFromValue(scope.database),
            timeoutMs: timeout_ms,
          }),
        );
        const heading = connectionBatchHeading(resolvedConfig, i + 1, resolvedConfigs.length);
        if ("error" in outcome) {
          if (outcome.error instanceof DatabaseStatsError && outcome.error.code === "UNSUPPORTED_DB_TYPE" && skipUnsupported) {
            skipped++;
            parts.push(
              prependConnectionProgress(`${heading}**Skipped** (${resolvedConfig.name}): ${outcome.error.message}`, outcome.progress),
            );
            continue;
          }
          failures++;
          if (resolvedConfigs.length === 1) {
            if (outcome.error instanceof DatabaseStatsError) {
              return toolErrorWithProgress(outcome.error.code, outcome.error.message, outcome.progress);
            }
            const msg = outcome.error instanceof Error ? outcome.error.message : String(outcome.error);
            return toolErrorWithProgress("DATABASE_STATS_ERROR", msg, outcome.progress);
          }
          const code = outcome.error instanceof DatabaseStatsError ? outcome.error.code : "DATABASE_STATS_ERROR";
          const msg = outcome.error instanceof Error ? outcome.error.message : String(outcome.error);
          parts.push(prependConnectionProgress(`${heading}**Error** (${resolvedConfig.name}): ${code}: ${msg}`, outcome.progress));
          continue;
        }
        successes++;
        parts.push(
          resolvedConfigs.length === 1
            ? labeledTextWithProgress(resolvedConfig, outcome.value, outcome.progress).content[0]!.text
            : prependConnectionProgress(`${heading}${outcome.value}`, outcome.progress),
        );
      }
      const summary = formatMcpBatchSummary(successes, skipped, failures, resolvedConfigs.length);
      const result = text(`${parts.join(MCP_BATCH_SEPARATOR)}${summary}`);
      return failures > 0 ? { ...result, isError: true } : result;
    },
  );

  server.tool(
    "dbx_get_database_report",
    "Get a comprehensive database report from system catalog views (information_schema, pg_catalog, sqlite_master): database summary, tables sorted by row estimate, column comments, and indexes — all instant catalog data, no COUNT queries. MCP returns text only (does not write files). CLI `dbx report` saves under `{cwd}/reports/` by default; use --no-save for stdout-only, -o to override path. Accepts connection ranges (e.g. 1-15); runs sequentially.",
    {
      connection_id: z.string().optional().describe(`Unique ID of the DBX connection, or list index/range from dbx_list_connections (e.g. 1, #2, 1-15)`),
      connection_name: z.string().optional().describe(CONNECTION_REF_HINT),
      database: z.string().optional().describe("Database name; for Dameng this is also accepted as a schema alias"),
      schema: z.string().optional().describe("Schema name (default: public for PostgreSQL, dbo for SQL Server, login user for Dameng)"),
      timeout_ms: z.number().int().min(1).optional().describe("Per-connection query timeout in milliseconds (aligned with CLI -t/--timeout)"),
      skip_unsupported: z
        .boolean()
        .optional()
        .describe("When true (default), unsupported db types return SKIPPED_UNSUPPORTED instead of hard error — aligned with CLI --skip-unsupported"),
      proxy_profile_id: z.string().optional().describe("One-shot: replace connection proxy with this saved profile ID or list index (#) for this request only"),
      proxy_profile_name: z.string().optional().describe("One-shot: replace connection proxy with this saved profile name or list index (#) for this request only"),
    },
    async ({ connection_id, connection_name, database, schema, timeout_ms, skip_unsupported, proxy_profile_id, proxy_profile_name }) => {
      const { configs, error } = await resolveConnectionsWithProxyOverride(backend, scope, isWebMode, {
        connection_id,
        connection_name,
        proxy_profile_id,
        proxy_profile_name,
      });
      if (error) return error;
      const resolvedConfigs = configs!;
      const skipUnsupported = skip_unsupported !== false;
      const parts: string[] = [];
      let successes = 0;
      let skipped = 0;
      let failures = 0;
      for (let i = 0; i < resolvedConfigs.length; i++) {
        const resolvedConfig = resolvedConfigs[i]!;
        const outcome = await runConnectingTool(resolvedConfig, () =>
          fetchDatabaseReport(backend, resolvedConfig, {
            database: database ?? scope.database,
            schema,
            redisDb: redisDbFromValue(database) ?? redisDbFromValue(scope.database),
            timeoutMs: timeout_ms,
          }),
        );
        const heading = connectionBatchHeading(resolvedConfig, i + 1, resolvedConfigs.length);
        if ("error" in outcome) {
          if (outcome.error instanceof DatabaseStatsError && outcome.error.code === "UNSUPPORTED_DB_TYPE" && skipUnsupported) {
            skipped++;
            parts.push(
              prependConnectionProgress(`${heading}**Skipped** (${resolvedConfig.name}): ${outcome.error.message}`, outcome.progress),
            );
            continue;
          }
          failures++;
          if (resolvedConfigs.length === 1) {
            if (outcome.error instanceof DatabaseStatsError) {
              return toolErrorWithProgress(outcome.error.code, outcome.error.message, outcome.progress);
            }
            const msg = outcome.error instanceof Error ? outcome.error.message : String(outcome.error);
            return toolErrorWithProgress("DATABASE_REPORT_ERROR", msg, outcome.progress);
          }
          const code = outcome.error instanceof DatabaseStatsError ? outcome.error.code : "DATABASE_REPORT_ERROR";
          const msg = outcome.error instanceof Error ? outcome.error.message : String(outcome.error);
          parts.push(prependConnectionProgress(`${heading}**Error** (${resolvedConfig.name}): ${code}: ${msg}`, outcome.progress));
          continue;
        }
        successes++;
        parts.push(
          resolvedConfigs.length === 1
            ? labeledTextWithProgress(resolvedConfig, outcome.value, outcome.progress).content[0]!.text
            : prependConnectionProgress(`${heading}${outcome.value}`, outcome.progress),
        );
      }
      const summary = formatMcpBatchSummary(successes, skipped, failures, resolvedConfigs.length);
      const result = text(`${parts.join(MCP_BATCH_SEPARATOR)}${summary}`);
      return failures > 0 ? { ...result, isError: true } : result;
    },
  );

  server.tool(
    "dbx_execute_query",
    "Execute a SQL query on a database connection (max 100 rows returned). Accepts connection ranges (e.g. 1-15); runs sequentially.",
    {
      connection_id: z.string().optional().describe(`Unique ID of the DBX connection, or list index/range from dbx_list_connections (e.g. 1, #2, 1-15)`),
      connection_name: z.string().optional().describe(CONNECTION_REF_HINT),
      database: z.string().optional().describe("Database name"),
      sql: z.string().describe("SQL query to execute"),
      timeout_ms: z.number().int().min(1).optional().describe("Query timeout in milliseconds (aligned with CLI -t/--timeout)"),
      proxy_profile_id: z.string().optional().describe("One-shot: replace connection proxy with this saved profile ID or list index (#) for this request only"),
      proxy_profile_name: z.string().optional().describe("One-shot: replace connection proxy with this saved profile name or list index (#) for this request only"),
    },
    async ({ connection_id, connection_name, database, sql, timeout_ms, proxy_profile_id, proxy_profile_name }) => {
      logSqlDiagnostic("dbx_execute_query", sql, { connection_id, connection_name, database });
      const { configs, error } = await resolveConnectionsWithProxyOverride(backend, scope, isWebMode, {
        connection_id,
        connection_name,
        proxy_profile_id,
        proxy_profile_name,
      });
      if (error) return error;
      const resolvedConfigs = configs!;
      if (resolvedConfigs.some((c) => c.db_type === "redis")) {
        return toolError("REDIS_COMMAND_REQUIRED", "Redis connections do not accept SQL through dbx_execute_query. Use dbx_execute_redis_command with a Redis command such as GET key or INFO.");
      }
      const hashLineComments = resolvedConfigs.some((c) => supportsHashLineComments(c.db_type));
      const safetyOptions = { ...sqlSafetyFromEnv(), allowMultipleStatements: true, hashLineComments };
      for (const scopedConfig of resolvedConfigs) {
        if (scopedConfig.db_type !== "mongodb") {
          const safety = evaluateSqlSafety(sql, { ...safetyOptions, hashLineComments: supportsHashLineComments(scopedConfig.db_type) });
          if (!safety.allowed) return toolError("SQL_BLOCKED", safety.reason ?? "SQL blocked.");
          const production = assessProductionSql(sql, scopedConfig, database ?? scope.database ?? scopedConfig.database);
          if (production.active && production.isMutation) {
            return toolError("PRODUCTION_WRITE_BLOCKED", "MCP cannot execute writes against a production database. Return the SQL for a user to review and run in DBX.");
          }
        } else if (isProductionDatabase(scopedConfig, database ?? scope.database ?? scopedConfig.database) && isLikelyMongoMutation(sql)) {
          return toolError("PRODUCTION_WRITE_BLOCKED", "MCP cannot execute writes against a production database. Return the command for a user to review and run in DBX.");
        }
      }
      const parts: string[] = [];
      let failures = 0;
      for (let i = 0; i < resolvedConfigs.length; i++) {
        const scopedConfig = resolvedConfigs[i]!;
        const outcome = await runConnectingTool(scopedConfig, async () => {
          const statements = scopedConfig.db_type === "mongodb" ? [sql] : splitSqlStatements(sql, { hashLineComments: supportsHashLineComments(scopedConfig.db_type) });
          const results: QueryResult[] = [];
          const queryOptions = timeout_ms !== undefined ? { timeoutMs: timeout_ms } : undefined;
          for (const statement of statements) {
            results.push(await backend.executeQuery(withDatabase(scopedConfig, database ?? scope.database), statement, queryOptions));
          }
          return results;
        });
        const heading = connectionBatchHeading(scopedConfig, i + 1, resolvedConfigs.length);
        if ("error" in outcome) {
          failures++;
          const msg = outcome.error instanceof Error ? outcome.error.message : String(outcome.error);
          if (resolvedConfigs.length === 1) return toolErrorWithProgress("QUERY_ERROR", msg, outcome.progress);
          parts.push(prependConnectionProgress(`${heading}**Error** (${scopedConfig.name}): QUERY_ERROR: ${msg}`, outcome.progress));
          continue;
        }
        const { value: results, progress } = outcome;
        const body =
          results.length === 1
            ? formatQueryToolResult(results[0]!).content[0]!.text
            : results.map((result, index) => formatQueryToolResult(result, `Statement ${index + 1}`).content[0]!.text).join("\n\n");
        parts.push(
          resolvedConfigs.length === 1
            ? labeledTextWithProgress(scopedConfig, body, progress).content[0]!.text
            : prependConnectionProgress(`${heading}${body}`, progress),
        );
      }
      const summary = formatMcpBatchSummary(resolvedConfigs.length - failures, 0, failures, resolvedConfigs.length);
      const result = text(`${parts.join(MCP_BATCH_SEPARATOR)}${summary}`);
      return failures > 0 ? { ...result, isError: true } : result;
    },
  );

  server.tool(
    "dbx_execute_redis_command",
    "Execute a Redis command on a Redis connection",
    {
      connection_id: z.string().optional().describe("Unique ID of the DBX connection, or list index (#) from dbx_list_connections (e.g. 1 or #2)"),
      connection_name: z.string().optional().describe("Name of the DBX Redis connection, or list index (#) from dbx_list_connections"),
      db: z.number().int().min(0).optional().describe("Redis logical database number (default: scoped/default database or 0)"),
      command: z.string().describe("Redis command to execute, for example: GET mykey, INFO, or DBSIZE"),
    },
    async ({ connection_id, connection_name, db, command }) => {
      const { config, error } = await resolveConnection(backend, scope, connection_id, connection_name);
      if (error) return error;
      const scopedConfig = config!;
      if (scopedConfig.db_type !== "redis") {
        return toolError("INVALID_CONNECTION_TYPE", `Connection "${scopedConfig.name}" is ${scopedConfig.db_type}, not Redis.`);
      }
      if (!backend.executeRedisCommand) {
        return toolError("UNSUPPORTED_BACKEND", "This DBX backend does not support Redis command execution.");
      }
      const safety = evaluateRedisCommandSafety(command, sqlSafetyFromEnv());
      if (!safety.allowed) return toolError("REDIS_COMMAND_BLOCKED", safety.reason ?? "Redis command blocked.");
      if (isProductionDatabase(scopedConfig, String(defaultRedisDb(scopedConfig, scope, db))) && safety.safety !== "allowed") {
        return toolError("PRODUCTION_WRITE_BLOCKED", "MCP cannot execute write or dangerous Redis commands against a production database.");
      }
      const outcome = await runConnectingTool(scopedConfig, () =>
        backend.executeRedisCommand!(scopedConfig, defaultRedisDb(scopedConfig, scope, db), command, {
          skipSafetyCheck: safety.skipSafetyCheck,
        }),
      );
      if ("error" in outcome) {
        const msg = outcome.error instanceof Error ? outcome.error.message : String(outcome.error);
        return toolErrorWithProgress("REDIS_COMMAND_ERROR", msg, outcome.progress);
      }
      return labeledTextWithProgress(scopedConfig, formatRedisCommandToolResult(outcome.value).content[0].text, outcome.progress);
    },
  );

  server.tool(
    "dbx_get_schema_context",
    "Get compact table and column context for writing SQL. Accepts connection ranges (e.g. 1-15); runs sequentially.",
    {
      connection_id: z.string().optional().describe(`Unique ID of the DBX connection, or list index/range from dbx_list_connections (e.g. 1, #2, 1-15)`),
      connection_name: z.string().optional().describe(CONNECTION_REF_HINT),
      database: z.string().optional().describe("Database name"),
      schema: z.string().optional().describe("Schema name (default: public for PostgreSQL)"),
      tables: z.array(z.string()).optional().describe("Specific table names to include"),
      max_tables: z.number().int().min(1).max(20).default(8).describe("Maximum number of tables to include"),
    },
    async ({ connection_id, connection_name, database, schema, tables, max_tables }) => {
      const { configs, error } = await resolveConnections(backend, scope, connection_id, connection_name);
      if (error) return error;
      const resolvedConfigs = configs!;
      const parts: string[] = [];
      let failures = 0;
      for (let i = 0; i < resolvedConfigs.length; i++) {
        const resolvedConfig = resolvedConfigs[i]!;
        const outcome = await runConnectingTool(resolvedConfig, () =>
          buildSchemaContext(backend, withDatabase(resolvedConfig, database ?? scope.database), {
            schema,
            tables,
            maxTables: max_tables,
          }),
        );
        const heading = connectionBatchHeading(resolvedConfig, i + 1, resolvedConfigs.length);
        if ("error" in outcome) {
          failures++;
          const msg = outcome.error instanceof Error ? outcome.error.message : String(outcome.error);
          if (resolvedConfigs.length === 1) return toolErrorWithProgress("SCHEMA_CONTEXT_ERROR", msg, outcome.progress);
          parts.push(prependConnectionProgress(`${heading}**Error** (${resolvedConfig.name}): SCHEMA_CONTEXT_ERROR: ${msg}`, outcome.progress));
          continue;
        }
        const { value: context, progress } = outcome;
        const body = context.tables.length === 0 ? "No matching tables found." : formatSchemaContext(context);
        parts.push(
          resolvedConfigs.length === 1
            ? labeledTextWithProgress(resolvedConfig, body, progress).content[0]!.text
            : prependConnectionProgress(`${heading}${body}`, progress),
        );
      }
      const summary = formatMcpBatchSummary(resolvedConfigs.length - failures, 0, failures, resolvedConfigs.length);
      const result = text(`${parts.join(MCP_BATCH_SEPARATOR)}${summary}`);
      return failures > 0 ? { ...result, isError: true } : result;
    },
  );

  if (!scoped) {
    server.tool(
      "dbx_add_connection",
      "Add a new database connection to DBX",
      {
        name: z.string().describe("Connection name"),
        db_type: z.string().describe(DBX_CONNECTION_TYPE_DESCRIPTION),
        host: z.string().describe("Database host; for cloudflare-d1, use the Cloudflare Account ID"),
        port: z.number().optional().describe("Database port (TDengine defaults to 6041, IoTDB defaults to 6667, XuguDB defaults to 5138)"),
        username: z.string().default("").describe("Username"),
        password: z.string().default("").describe("Password; for cloudflare-d1, use the API Token"),
        database: z.string().optional().describe("Default database name; for cloudflare-d1, use the D1 Database ID"),
        ssl: z.boolean().default(false).describe("Enable SSL"),
        driver_profile: z.string().optional().describe("Driver profile (e.g. 'gbase8a', 'gbase8s')"),
        proxy_enabled: z.boolean().default(false).describe("Enable SOCKS5 or HTTP proxy tunnel for this connection"),
        proxy_type: z.enum(["socks5", "http"]).default("socks5").describe("Proxy protocol (default: socks5)"),
        proxy_host: z.string().optional().describe("Proxy server host (required when proxy_enabled is true)"),
        proxy_port: z.number().int().min(1).max(65535).optional().describe("Proxy server port (default: 1080)"),
        proxy_username: z.string().optional().describe("Proxy authentication username"),
        proxy_password: z.string().optional().describe("Proxy authentication password"),
        proxy_profile_id: z.string().optional().describe("ID of a saved proxy tunnel profile, or list index (#) from dbx_list_proxies (e.g. 1 or #2)"),
        proxy_profile_name: z.string().optional().describe("Name of a saved proxy tunnel profile, or list index (#) from dbx_list_proxies"),
      },
      async ({
        name,
        db_type,
        host,
        port,
        username,
        password,
        database,
        ssl,
        driver_profile,
        proxy_enabled,
        proxy_type,
        proxy_host,
        proxy_port,
        proxy_username,
        proxy_password,
        proxy_profile_id,
        proxy_profile_name,
      }) => {
        const existing = await backend.findConnection(name);
        if (existing) return text(`Connection "${name}" already exists.`);
        const DEFAULT_PORTS: Record<string, number> = {
          kwdb: 26257,
          rqlite: 4001,
          "cloudflare-d1": 443,
          tdengine: 6041,
          oscar: 2003,
          iotdb: 6667,
          xugu: 5138,
        };
        const resolvedPort = port ?? DEFAULT_PORTS[db_type] ?? (FILE_CAPABLE_CONNECTION_TYPES.has(db_type) ? 0 : undefined);
        if (resolvedPort === undefined) return text("Port is required for this database type.");

        const profileRef = hasProxyProfileRef({ proxy_profile_id, proxy_profile_name });
        const inlineProxy = hasInlineProxyParams({ proxy_enabled, proxy_host, proxy_port, proxy_username, proxy_password });
        if (profileRef && inlineProxy) {
          return toolError(
            "PROXY_CONFLICT",
            "Cannot mix saved proxy reference (proxy_profile_id/proxy_profile_name) with inline proxy settings (proxy_enabled, proxy_host, etc.). Use one mode only.",
          );
        }
        if (profileRef && proxy_profile_id?.trim() && proxy_profile_name?.trim()) {
          return toolError("PROXY_CONFLICT", "Specify either proxy_profile_id or proxy_profile_name, not both.");
        }

        const baseConfig: Record<string, unknown> = {
          name,
          db_type,
          host,
          port: resolvedPort,
          username,
          password,
          database,
          ssl,
          driver_profile,
          ssh_enabled: false,
        };

        let savedProxyLabel = "";
        if (profileRef) {
          const profiles = await loadTunnelProfiles(isWebMode);
          const proxies = profiles.filter((profile) => profile.type === "proxy");
          if (proxy_profile_name?.trim() && !proxy_profile_id?.trim()) {
            const matches = findProxyProfilesByName(profiles, proxy_profile_name);
            if (matches.length > 1) {
              const lines = matches.map((item) => {
                const proxyIdx = proxies.indexOf(item);
                const num = proxyIdx >= 0 ? proxyIdx + 1 : "?";
                return `- #${num} ${item.id}: ${proxyProfileSummary(item)}`;
              });
              return toolError("AMBIGUOUS_PROXY_PROFILE", `Multiple proxy profiles named "${proxy_profile_name}". Specify proxy_profile_id or list index (#):\n${lines.join("\n")}`);
            }
          }
          const profile = findProxyProfile(profiles, { proxy_profile_id, proxy_profile_name });
          if (!profile) {
            return toolError("PROXY_PROFILE_NOT_FOUND", "Proxy profile not found. Use dbx_list_proxies to see saved profiles from DBX Settings > Tunnels.");
          }
          savedProxyLabel = profile.name?.trim() || profile.id;
          Object.assign(baseConfig, applyProxyProfileOverride(baseConfig, profile));
        } else if (proxy_enabled) {
          if (!proxy_host?.trim()) return text("proxy_host is required when proxy_enabled is true.");
          Object.assign(baseConfig, {
            proxy_enabled,
            proxy_type,
            proxy_host: proxy_host.trim(),
            proxy_port: proxy_port ?? 1080,
            proxy_username: proxy_username?.trim() || undefined,
            proxy_password: proxy_password ?? undefined,
          });
        }

        const config = await backend.addConnection(baseConfig as Omit<ConnectionConfig, "id">);
        await notifyReload();
        const proxyNote = savedProxyLabel ? ` using saved proxy profile "${savedProxyLabel}"` : "";
        return text(`Connection "${config.name}" added (id: ${config.id})${proxyNote}.`);
      },
    );

    server.tool(
      "dbx_remove_connection",
      "Remove a database connection from DBX",
      {
        connection_name: z.string().describe("Name of the connection to remove, or list index (#) from dbx_list_connections"),
        connection_id: z.string().optional().describe("Unique ID of the DBX connection, or list index (#) from dbx_list_connections"),
      },
      async ({ connection_name, connection_id }) => {
        const connections = await backend.loadConnections();
        let target: ConnectionConfig | undefined;

        if (connection_id?.trim()) {
          const trimmed = connection_id.trim();
          target = connections.find((c) => c.id === trimmed);
          if (!target) {
            const listIndex = parseListIndex(trimmed);
            if (listIndex !== undefined) target = resolveConnectionByIndex(connections, listIndex);
          }
          if (!target) return toolError("CONNECTION_NOT_FOUND", `Connection with id "${connection_id}" not found.`);
        } else {
          const trimmed = connection_name.trim();
          const matching = connections.filter((c) => c.name.toLowerCase() === trimmed.toLowerCase());
          if (matching.length > 1) {
            const lines = matching.map((c) => {
              const idx = connections.indexOf(c);
              const num = idx >= 0 ? idx + 1 : "?";
              return `- #${num} ${c.id}: ${c.db_type} @ ${c.host}:${c.port}`;
            });
            return toolError("AMBIGUOUS_CONNECTION", `Multiple connections found with name "${connection_name}". Please specify connection_id or list index (#):\n${lines.join("\n")}`);
          }
          target = matching[0];
          if (!target) {
            const listIndex = parseListIndex(trimmed);
            if (listIndex !== undefined) target = resolveConnectionByIndex(connections, listIndex);
          }
          if (!target) return toolError("CONNECTION_NOT_FOUND", `Connection "${connection_name}" not found.`);
        }

        if (backend.removeConnectionById) {
          const removed = await backend.removeConnectionById(target.id);
          if (!removed) return toolError("CONNECTION_NOT_FOUND", `Connection with id "${target.id}" not found.`);
          await notifyReload();
          return text(`Connection "${target.name}" (id: ${target.id}) removed.`);
        }
        const removed = await backend.removeConnection(target.name);
        if (!removed) return toolError("CONNECTION_NOT_FOUND", `Connection "${target.name}" could not be removed.`);
        await notifyReload();
        return text(`Connection "${target.name}" (id: ${target.id}) removed.`);
      },
    );
  }

  // Desktop-only tools: open table and execute-and-show require the Tauri bridge
  if (!isWebMode && !scoped) {
    server.tool(
      "dbx_open_table",
      "Open a table in DBX desktop app UI. Requires DBX to be running.",
      {
        connection_id: z.string().optional().describe("Unique ID of the DBX connection, or list index (#) from dbx_list_connections (e.g. 1 or #2)"),
        connection_name: z.string().optional().describe("Name of the DBX connection, or list index (#) from dbx_list_connections"),
        table: z.string().describe("Table name to open"),
        database: z.string().optional().describe("Database name"),
        schema: z.string().optional().describe("Schema name"),
      },
      async ({ connection_id, connection_name, table, database, schema }) => {
        const { config, error } = await resolveConnection(backend, scope, connection_id, connection_name);
        if (error) return error;
        const resolvedConfig = config!;
        const outcome = await runConnectingTool(resolvedConfig, () =>
          bridgeRequest("/open-table", { connection_id: resolvedConfig.id, connection_name: resolvedConfig.name, table, database, schema }, `Opened ${table} in DBX`),
        );
        if ("error" in outcome) {
          const msg = outcome.error instanceof Error ? outcome.error.message : String(outcome.error);
          const code = msg.startsWith("DBX is not running") ? "DBX_NOT_RUNNING" : "OPEN_TABLE_ERROR";
          return toolErrorWithProgress(code, msg, outcome.progress);
        }
        return toolResultWithProgress(outcome.value, outcome.progress);
      },
    );

    server.tool(
      "dbx_execute_and_show",
      "Execute a SQL query in DBX desktop app UI and show results there. Requires DBX to be running.",
      {
        connection_id: z.string().optional().describe("Unique ID of the DBX connection, or list index (#) from dbx_list_connections (e.g. 1 or #2)"),
        connection_name: z.string().optional().describe("Name of the DBX connection, or list index (#) from dbx_list_connections"),
        sql: z.string().describe("SQL query to execute"),
        database: z.string().optional().describe("Database name"),
      },
      async ({ connection_id, connection_name, sql, database }) => {
        const { config, error } = await resolveConnection(backend, scope, connection_id, connection_name);
        if (error) return error;
        const resolvedConfig = config!;
        const safetyOptions = sqlSafetyFromEnv();
        if (resolvedConfig.db_type === "mongodb") {
          const aggregate = parseMongoAggregateCommand(sql);
          if (aggregate) {
            const safety = evaluateMongoAggregateSafety(aggregate, safetyOptions);
            if (!safety.allowed) return toolError("SQL_BLOCKED", safety.reason ?? "Query blocked.");
          }
        } else {
          const hashLineComments = supportsHashLineComments(config?.db_type);
          const safety = evaluateSqlSafety(sql, { ...safetyOptions, allowMultipleStatements: true, hashLineComments });
          if (!safety.allowed) return toolError("SQL_BLOCKED", safety.reason ?? "SQL blocked.");
        }
        if (config?.db_type === "mongodb") {
          if (isProductionDatabase(resolvedConfig, database ?? scope.database ?? resolvedConfig.database) && isLikelyMongoMutation(sql)) {
            return toolError("PRODUCTION_WRITE_BLOCKED", "MCP cannot send writes against a production database to DBX.");
          }
        } else {
          const production = assessProductionSql(sql, resolvedConfig, database ?? scope.database ?? resolvedConfig.database);
          if (production.active && production.isMutation) {
            return toolError("PRODUCTION_WRITE_BLOCKED", "MCP cannot send writes against a production database to DBX.");
          }
        }
        // MongoDB shell commands bypass the SQL safety evaluator; pass MCP
        // safety flags to the desktop executor for command-aware gating.
        logSqlDiagnostic("dbx_execute_in_app", sql, { connection_id: config!.id, connection_name: config!.name, database });
        const outcome = await runConnectingTool(resolvedConfig, () =>
          bridgeRequest(
            "/execute-query",
            {
              connection_id: resolvedConfig.id,
              connection_name: resolvedConfig.name,
              sql,
              database,
              allow_writes: safetyOptions.allowWrites,
              allow_dangerous: safetyOptions.allowDangerous,
            },
            "Query sent to DBX",
          ),
        );
        if ("error" in outcome) {
          const msg = outcome.error instanceof Error ? outcome.error.message : String(outcome.error);
          const code = msg.startsWith("DBX is not running") ? "DBX_NOT_RUNNING" : "EXECUTE_AND_SHOW_ERROR";
          return toolErrorWithProgress(code, msg, outcome.progress);
        }
        return toolResultWithProgress(outcome.value, outcome.progress);
      },
    );
  }

  return server;
}

async function bridgeRequest(path: string, body: Record<string, unknown>, successMsg: string) {
  const res = await postBridge(path, body);
  if (res.ok) return text(successMsg);
  const message = res.text.startsWith("DBX is not running") ? res.text : `Failed: ${res.text}`;
  throw new Error(message);
}

async function main() {
  const backend = await createBackend();
  const server = createDbxMcpServer(backend);
  const transport = new StdioServerTransport();
  await server.connect(transport);
}

if (isMainModule(import.meta.url, process.argv[1])) {
  main().catch((e) => {
    console.error("MCP Server failed to start:", e);
    process.exit(1);
  });
}
