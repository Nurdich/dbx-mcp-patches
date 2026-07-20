import type { Backend } from "./backend.js";
import type { ConnectionConfig } from "./connections.js";
import type { QueryResult } from "./database.js";
import { withConnectionStage } from "./connection-log.js";
import { formatCell, mdTable } from "./format.js";

const MYSQL_STATS_TYPES = new Set(["mysql", "doris", "starrocks", "manticoresearch"]);
const POSTGRES_STATS_TYPES = new Set(["postgres", "redshift", "gaussdb", "kwdb", "opengauss", "questdb", "kingbase", "highgo", "vastbase", "dameng"]);
const SQLITE_STATS_TYPES = new Set(["sqlite", "rqlite"]);

/** Non-SQL / non-Redis / non-Mongo types: fail fast for stats/report (do not call bridge). */
export const NON_CATALOG_STATS_TYPES = new Set([
  "elasticsearch",
  "etcd",
  "neo4j",
  "cassandra",
  "milvus",
  "qdrant",
  "weaviate",
  "chromadb",
  "zookeeper",
  "mq",
  "kafka",
  "influxdb",
]);

/** Types that must not use information_schema catalog SQL builders (includes Redis/Mongo). */
export const UNSUPPORTED_STATS_TYPES = new Set(["redis", "mongodb", ...NON_CATALOG_STATS_TYPES]);

export function isNonCatalogStatsType(dbType: string): boolean {
  return NON_CATALOG_STATS_TYPES.has(dbType);
}

export function unsupportedStatsOverviewMessage(dbType: string, kind: "stats" | "report" = "stats"): string {
  const noun = kind === "report" ? "report" : "stats overview";
  return `Database ${noun} is not supported for ${dbType}. Supported: Redis, MongoDB, MySQL/MariaDB family, PostgreSQL family, SQLite/rqlite, and other SQL engines with information_schema.`;
}

const MYSQL_SYSTEM_DATABASES = ["information_schema", "mysql", "performance_schema", "sys"] as const;
const POSTGRES_SYSTEM_SCHEMAS = ["information_schema", "pg_catalog", "pg_toast"] as const;
const GENERIC_SYSTEM_SCHEMAS = ["information_schema", "pg_catalog", "mysql", "performance_schema", "sys"] as const;

const ROW_COUNT_FIELDS = ["rows_estimate", "TABLE_ROWS", "n_live_tup", "reltuples", "count", "nrecords", "Rows", "Docs"] as const;

export interface DatabaseStatsOptions {
  database?: string;
  schema?: string;
  redisDb?: number;
  timeoutMs?: number;
}

export interface CatalogStatsScope {
  database?: string;
  schema?: string;
}

function withDatabase(config: ConnectionConfig, database?: string): ConnectionConfig {
  return database === undefined ? config : { ...config, database };
}

export function metadataScope(
  config: ConnectionConfig,
  database?: string,
  schema?: string,
): { config: ConnectionConfig; schema?: string } {
  if (config.db_type !== "dameng") {
    return { config: withDatabase(config, database), schema };
  }

  const resolvedSchema = schema?.trim() || database?.trim() || config.username?.trim() || undefined;
  return { config, schema: resolvedSchema };
}

function sqlLiteral(value: string): string {
  return `'${value.replace(/'/g, "''")}'`;
}

function sqlInList(values: readonly string[]): string {
  return values.map(sqlLiteral).join(", ");
}

function defaultStatsSchema(dbType: string): string {
  if (dbType === "sqlserver") return "dbo";
  if (MYSQL_STATS_TYPES.has(dbType)) return "";
  return "public";
}

function parseRowCount(row: Record<string, unknown>): number | null {
  for (const field of ROW_COUNT_FIELDS) {
    if (!(field in row)) continue;
    const value = row[field];
    if (value === null || value === undefined || value === "") continue;
    const n = typeof value === "bigint" ? Number(value) : Number(value);
    if (Number.isFinite(n) && n >= 0) return n;
  }
  return null;
}

export function sortStatsRows<T extends Record<string, unknown>>(rows: T[]): T[] {
  return [...rows].sort((a, b) => {
    const aCount = parseRowCount(a);
    const bCount = parseRowCount(b);
    if (aCount === null && bCount === null) return 0;
    if (aCount === null) return 1;
    if (bCount === null) return -1;
    return bCount - aCount;
  });
}

export function resolveCatalogStatsScope(
  dbType: string,
  options: DatabaseStatsOptions,
  scopeValue: { config: ConnectionConfig; schema?: string },
): CatalogStatsScope {
  const explicitDatabase = options.database?.trim();
  const explicitSchema = options.schema?.trim();
  const configDatabase = scopeValue.config.database?.trim();

  if (dbType === "dameng") {
    return {
      database: explicitDatabase || configDatabase || undefined,
      schema: explicitSchema || explicitDatabase || scopeValue.schema || configDatabase,
    };
  }
  if (MYSQL_STATS_TYPES.has(dbType)) {
    return {
      database: explicitDatabase || configDatabase || undefined,
      schema: explicitSchema,
    };
  }
  return {
    database: explicitDatabase || configDatabase || undefined,
    schema: explicitSchema,
  };
}

function isAllUserScopesMode(dbType: string, catalogScope: CatalogStatsScope): boolean {
  if (MYSQL_STATS_TYPES.has(dbType)) return !catalogScope.database;
  if (POSTGRES_STATS_TYPES.has(dbType)) return !catalogScope.schema;
  return !catalogScope.schema;
}

export function formatStatBytes(value: unknown): string {
  const n = typeof value === "bigint" ? Number(value) : Number(value);
  if (!Number.isFinite(n) || n < 0) return "";
  if (n === 0) return "0 B";
  const units = ["B", "KB", "MB", "GB", "TB"];
  let size = n;
  let unit = 0;
  while (size >= 1024 && unit < units.length - 1) {
    size /= 1024;
    unit += 1;
  }
  const rounded = unit === 0 ? String(Math.round(size)) : size < 10 ? size.toFixed(1) : String(Math.round(size));
  return `${rounded} ${units[unit]}`;
}

export function buildCatalogStatsSql(dbType: string, scope: CatalogStatsScope = {}): string | null {
  const explicitDatabase = scope.database?.trim();
  const explicitSchema = scope.schema?.trim();

  if (MYSQL_STATS_TYPES.has(dbType)) {
    const systemDbs = sqlInList(MYSQL_SYSTEM_DATABASES);
    const dbFilter = explicitDatabase
      ? `TABLE_SCHEMA = ${sqlLiteral(explicitDatabase)}`
      : `TABLE_SCHEMA NOT IN (${systemDbs})`;
    const scopeColumn = explicitDatabase ? "" : "TABLE_SCHEMA AS database_name, ";
    const tableTypeFilter = explicitDatabase
      ? `TABLE_TYPE = 'BASE TABLE'`
      : `TABLE_TYPE IN ('BASE TABLE', 'VIEW')`;
    return `SELECT ${scopeColumn}TABLE_NAME AS name, TABLE_TYPE AS type, ENGINE AS engine, TABLE_ROWS AS rows_estimate, DATA_LENGTH AS data_bytes, INDEX_LENGTH AS index_bytes, (COALESCE(DATA_LENGTH, 0) + COALESCE(INDEX_LENGTH, 0)) AS total_bytes, TABLE_COMMENT AS comment FROM information_schema.TABLES WHERE ${dbFilter} AND ${tableTypeFilter}`;
  }
  if (POSTGRES_STATS_TYPES.has(dbType)) {
    const systemSchemas = sqlInList(POSTGRES_SYSTEM_SCHEMAS);
    if (explicitSchema) {
      const schemaLit = sqlLiteral(explicitSchema);
      return `SELECT t.table_name AS name, t.table_type AS type, NULL AS engine, COALESCE(st.n_live_tup, FLOOR(c.reltuples))::bigint AS rows_estimate, pg_relation_size(c.oid) AS data_bytes, GREATEST(pg_total_relation_size(c.oid) - pg_relation_size(c.oid), 0) AS index_bytes, pg_total_relation_size(c.oid) AS total_bytes, obj_description(c.oid, 'pg_class') AS comment FROM information_schema.tables t INNER JOIN pg_catalog.pg_class c ON c.relname = t.table_name INNER JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace AND n.nspname = t.table_schema LEFT JOIN pg_catalog.pg_stat_user_tables st ON st.schemaname = t.table_schema AND st.relname = t.table_name WHERE t.table_schema = ${schemaLit} AND t.table_type IN ('BASE TABLE', 'VIEW')`;
    }
    return `SELECT t.table_schema AS schema_name, t.table_name AS name, t.table_type AS type, NULL AS engine, COALESCE(st.n_live_tup, FLOOR(c.reltuples))::bigint AS rows_estimate, pg_relation_size(c.oid) AS data_bytes, GREATEST(pg_total_relation_size(c.oid) - pg_relation_size(c.oid), 0) AS index_bytes, pg_total_relation_size(c.oid) AS total_bytes, obj_description(c.oid, 'pg_class') AS comment FROM information_schema.tables t INNER JOIN pg_catalog.pg_class c ON c.relname = t.table_name INNER JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace AND n.nspname = t.table_schema LEFT JOIN pg_catalog.pg_stat_user_tables st ON st.schemaname = t.table_schema AND st.relname = t.table_name WHERE t.table_schema NOT IN (${systemSchemas}) AND t.table_type IN ('BASE TABLE', 'VIEW')`;
  }
  if (SQLITE_STATS_TYPES.has(dbType)) {
    return `SELECT name, type, NULL AS engine, NULL AS rows_estimate, NULL AS data_bytes, NULL AS index_bytes, NULL AS total_bytes, NULL AS comment FROM sqlite_master WHERE type IN ('table', 'view') AND name NOT LIKE 'sqlite_%'`;
  }
  if (UNSUPPORTED_STATS_TYPES.has(dbType)) return null;
  const schemaName = explicitSchema || defaultStatsSchema(dbType);
  if (schemaName) {
    return `SELECT table_name AS name, table_type AS type, NULL AS engine, NULL AS rows_estimate, NULL AS data_bytes, NULL AS index_bytes, NULL AS total_bytes, NULL AS comment FROM information_schema.tables WHERE table_schema = ${sqlLiteral(schemaName)} AND table_type IN ('BASE TABLE', 'VIEW')`;
  }
  return `SELECT table_name AS name, table_type AS type, NULL AS engine, NULL AS rows_estimate, NULL AS data_bytes, NULL AS index_bytes, NULL AS total_bytes, NULL AS comment FROM information_schema.tables WHERE table_schema NOT IN (${sqlInList(GENERIC_SYSTEM_SCHEMAS)}) AND table_type IN ('BASE TABLE', 'VIEW')`;
}

export function buildCatalogSummarySql(dbType: string, scope: CatalogStatsScope = {}): string | null {
  const explicitDatabase = scope.database?.trim();
  const explicitSchema = scope.schema?.trim();

  if (MYSQL_STATS_TYPES.has(dbType)) {
    if (!explicitDatabase) return null;
    return `SELECT SCHEMA_NAME AS database_name, DEFAULT_CHARACTER_SET_NAME AS charset, DEFAULT_COLLATION_NAME AS collation FROM information_schema.SCHEMATA WHERE SCHEMA_NAME = ${sqlLiteral(explicitDatabase)}`;
  }
  if (POSTGRES_STATS_TYPES.has(dbType)) {
    if (!explicitSchema) return null;
    return `SELECT current_database() AS database_name, ${sqlLiteral(explicitSchema)} AS schema_name, pg_size_pretty(pg_database_size(current_database())) AS database_size`;
  }
  return null;
}

function uniqueNonEmptyFieldCount(rows: Record<string, unknown>[], field: string): number {
  const seen = new Set<string>();
  for (const row of rows) {
    const value = row[field];
    if (value == null) continue;
    const text = String(value).trim();
    if (text) seen.add(text);
  }
  return seen.size;
}

function formatSummaryKeyValues(entries: Array<[string, unknown]>): string {
  return entries
    .filter(([, value]) => value !== undefined && value !== null && String(value).trim() !== "")
    .map(([key, value]) => `${key}: ${formatCell(value)}`)
    .join("\n");
}

export function deriveCatalogSummaryFromStats(
  dbType: string,
  scope: CatalogStatsScope,
  stats: QueryResult,
  config?: ConnectionConfig,
): string {
  const explicitDatabase = scope.database?.trim();
  const explicitSchema = scope.schema?.trim();
  const tableCount = stats.rows.length;

  if (MYSQL_STATS_TYPES.has(dbType) && explicitDatabase) {
    return formatSummaryKeyValues([
      ["database_name", explicitDatabase],
      ["table_count", tableCount],
    ]);
  }
  if (MYSQL_STATS_TYPES.has(dbType) && !explicitDatabase) {
    return formatSummaryKeyValues([
      ["database_count", uniqueNonEmptyFieldCount(stats.rows, "database_name")],
      ["table_count", tableCount],
    ]);
  }
  if (POSTGRES_STATS_TYPES.has(dbType) && !explicitSchema) {
    return formatSummaryKeyValues([
      ["database_name", config?.database?.trim() ?? ""],
      ["schema_count", uniqueNonEmptyFieldCount(stats.rows, "schema_name")],
      ["table_count", tableCount],
    ]);
  }
  if (SQLITE_STATS_TYPES.has(dbType)) {
    return formatSummaryKeyValues([
      ["database_name", "main"],
      ["object_count", tableCount],
    ]);
  }
  return "";
}

export function formatStatsOverviewTable(result: QueryResult): string {
  const sorted = sortStatsRows(result.rows);
  const hasDatabase = sorted.some((row) => row.database_name != null && String(row.database_name).trim() !== "");
  const hasSchema = !hasDatabase && sorted.some((row) => row.schema_name != null && String(row.schema_name).trim() !== "");

  const headers = hasDatabase
    ? ["Database", "Name", "Type", "Engine", "Rows (est.)", "Data", "Index", "Total", "Comment"]
    : hasSchema
      ? ["Schema", "Name", "Type", "Engine", "Rows (est.)", "Data", "Index", "Total", "Comment"]
      : ["Name", "Type", "Engine", "Rows (est.)", "Data", "Index", "Total", "Comment"];

  const rows = sorted.map((row) => {
    const cells = [
      formatCell(row.name),
      formatCell(row.type),
      formatCell(row.engine),
      formatCell(row.rows_estimate),
      formatStatBytes(row.data_bytes),
      formatStatBytes(row.index_bytes),
      formatStatBytes(row.total_bytes),
      formatCell(row.comment),
    ];
    if (hasDatabase) return [formatCell(row.database_name), ...cells];
    if (hasSchema) return [formatCell(row.schema_name), ...cells];
    return cells;
  });

  return mdTable(headers, rows);
}

function formatSummaryLines(result: QueryResult): string {
  if (result.rows.length === 0) return "";
  const row = result.rows[0];
  const lines = result.columns.map((column) => `${column}: ${formatCell(row[column])}`);
  return lines.join("\n");
}

function parseRedisInfoSections(infoText: string): Record<string, Record<string, string>> {
  const sections: Record<string, Record<string, string>> = {};
  let section = "server";
  for (const line of infoText.split(/\r?\n/)) {
    if (!line || line.startsWith("#")) {
      const title = line.replace(/^#\s*/, "").trim().toLowerCase().replace(/\s+/g, "_");
      if (title) section = title;
      continue;
    }
    const idx = line.indexOf(":");
    if (idx <= 0) continue;
    const key = line.slice(0, idx).trim();
    const value = line.slice(idx + 1).trim();
    sections[section] ??= {};
    sections[section][key] = value;
  }
  return sections;
}

function defaultRedisDb(config: ConnectionConfig, options: DatabaseStatsOptions): number {
  if (options.redisDb !== undefined) return options.redisDb;
  const fromConfig = config.database?.trim();
  if (fromConfig) {
    const db = Number(fromConfig);
    if (Number.isInteger(db) && db >= 0) return db;
  }
  return 0;
}

async function fetchRedisDatabaseStats(backend: Backend, config: ConnectionConfig, options: DatabaseStatsOptions): Promise<string> {
  if (!backend.executeRedisCommand) {
    return "Redis command execution is not available in this backend.";
  }
  const redisDb = defaultRedisDb(config, options);
  const infoResult = await backend.executeRedisCommand(config, redisDb, "INFO");
  const dbsizeResult = await backend.executeRedisCommand(config, redisDb, "DBSIZE");
  const infoText = typeof infoResult.value === "string" ? infoResult.value : String(infoResult.value ?? "");
  const sections = parseRedisInfoSections(infoText);
  const memory = sections.memory ?? {};
  const keyspace = sections.keyspace ?? {};
  const server = sections.server ?? {};
  const summaryRows = [
    ["redis_version", server.redis_version ?? ""],
    ["role", server.role ?? ""],
    ["used_memory_human", memory.used_memory_human ?? ""],
    ["used_memory_peak_human", memory.used_memory_peak_human ?? ""],
    ["connected_clients", sections.clients?.connected_clients ?? ""],
    ["db", String(redisDb)],
    ["dbsize", String(dbsizeResult.value ?? "")],
  ];
  const parts = ["Summary", mdTable(["Metric", "Value"], summaryRows)];
  const keyspaceRows = Object.entries(keyspace).map(([dbKey, value]) => [dbKey, value]);
  if (keyspaceRows.length > 0) {
    parts.push("", "Keyspace", mdTable(["DB", "Stats"], keyspaceRows));
  }
  return parts.join("\n");
}

async function fetchMongoDatabaseStats(backend: Backend, config: ConnectionConfig, schema?: string): Promise<string> {
  const tables = await backend.listTables(config, schema);
  if (tables.length === 0) return "No collections found.";
  const statRows: Array<Record<string, unknown>> = [];
  const limit = Math.min(tables.length, 50);
  for (let i = 0; i < limit; i += 1) {
    const table = tables[i]!;
    try {
      const result = await backend.executeQuery(config, `db.${table.name}.stats()`);
      const statRow = result.rows[0] ?? {};
      statRows.push({
        name: table.name,
        type: table.type,
        count: statRow.count ?? statRow.nrecords ?? null,
        size: statRow.size ?? statRow.avgObjSize,
        totalIndexSize: statRow.totalIndexSize,
        storageSize: statRow.storageSize,
        storageEngine: statRow.storageEngine ?? "",
      });
    } catch {
      statRows.push({
        name: table.name,
        type: table.type,
        count: null,
        unavailable: true,
      });
    }
  }
  const sorted = sortStatsRows(statRows);
  const rows = sorted.map((statRow) => {
    if (statRow.unavailable) {
      return [String(statRow.name), String(statRow.type), "", "", "", "", "", "stats unavailable"];
    }
    return [
      String(statRow.name),
      String(statRow.type),
      "",
      formatCell(statRow.count),
      formatStatBytes(statRow.size),
      formatStatBytes(statRow.totalIndexSize),
      formatStatBytes(statRow.storageSize),
      formatCell(statRow.storageEngine),
    ];
  });
  const header = tables.length > limit ? `\nShowing ${limit} of ${tables.length} collections (catalog stats).\n` : "";
  return `${header}${mdTable(["Name", "Type", "Engine", "Docs (est.)", "Data", "Index", "Storage", "Storage Engine"], rows)}`;
}

export class DatabaseStatsError extends Error {
  readonly code: string;

  constructor(code: string, message: string) {
    super(message);
    this.code = code;
    this.name = "DatabaseStatsError";
  }
}

export async function fetchDatabaseStats(backend: Backend, config: ConnectionConfig, options: DatabaseStatsOptions = {}): Promise<string> {
  return withConnectionStage("Fetching catalog stats", async () => {
    const scopeValue = metadataScope(config, options.database, options.schema);
    const dbType = scopeValue.config.db_type;

  if (dbType === "redis") {
    return fetchRedisDatabaseStats(backend, scopeValue.config, options);
  }
  if (dbType === "mongodb") {
    return fetchMongoDatabaseStats(backend, scopeValue.config, scopeValue.schema);
  }
  if (isNonCatalogStatsType(dbType)) {
    throw new DatabaseStatsError("UNSUPPORTED_DB_TYPE", unsupportedStatsOverviewMessage(dbType, "stats"));
  }

  const catalogScope = resolveCatalogStatsScope(dbType, options, scopeValue);
  const statsSql = buildCatalogStatsSql(dbType, catalogScope);
  if (!statsSql) {
    throw new DatabaseStatsError("UNSUPPORTED_DB_TYPE", unsupportedStatsOverviewMessage(dbType, "stats"));
  }

  const parts: string[] = [];
  const queryOptions = options.timeoutMs !== undefined ? { timeoutMs: options.timeoutMs } : undefined;

  const stats = await backend.executeQuery(scopeValue.config, statsSql, queryOptions);
  const summaryText = deriveCatalogSummaryFromStats(dbType, catalogScope, stats, scopeValue.config);
  if (summaryText) parts.push("Summary", summaryText);
  if (stats.rows.length === 0) {
    parts.push(parts.length > 0 ? "" : "", "No tables found in catalog.");
  } else {
    if (parts.length > 0) parts.push("");
    const scopeLabel = isAllUserScopesMode(dbType, catalogScope) ? " (all user scopes)" : "";
    parts.push(`Tables (${stats.row_count})${scopeLabel}`, formatStatsOverviewTable(stats));
  }

  return parts.join("\n");
  });
}
