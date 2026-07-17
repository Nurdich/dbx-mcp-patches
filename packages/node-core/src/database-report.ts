import { join } from "node:path";
import type { Backend } from "./backend.js";
import type { ConnectionConfig } from "./connections.js";
import type { QueryResult } from "./database.js";
import { withConnectionStage } from "./connection-log.js";
import { formatCell, mdTable } from "./format.js";
import {
  DatabaseStatsError,
  UNSUPPORTED_STATS_TYPES,
  buildCatalogStatsSql,
  buildCatalogSummarySql,
  deriveCatalogSummaryFromStats,
  fetchDatabaseStats,
  formatStatsOverviewTable,
  isNonCatalogStatsType,
  metadataScope,
  resolveCatalogStatsScope,
  unsupportedStatsOverviewMessage,
  type CatalogStatsScope,
  type DatabaseStatsOptions,
} from "./database-stats.js";

const MYSQL_STATS_TYPES = new Set(["mysql", "doris", "starrocks", "manticoresearch"]);
const POSTGRES_STATS_TYPES = new Set(["postgres", "redshift", "gaussdb", "kwdb", "opengauss", "questdb", "kingbase", "highgo", "vastbase", "dameng"]);
const SQLITE_STATS_TYPES = new Set(["sqlite", "rqlite"]);

const MYSQL_SYSTEM_DATABASES = ["information_schema", "mysql", "performance_schema", "sys"] as const;
const POSTGRES_SYSTEM_SCHEMAS = ["information_schema", "pg_catalog", "pg_toast"] as const;

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

function formatSummaryLines(result: QueryResult): string {
  if (result.rows.length === 0) return "";
  const row = result.rows[0];
  const lines = result.columns.map((column) => `${column}: ${formatCell(row[column])}`);
  return lines.join("\n");
}

export function buildCatalogIndexSql(dbType: string, scope: CatalogStatsScope = {}): string | null {
  const explicitDatabase = scope.database?.trim();
  const explicitSchema = scope.schema?.trim();

  if (MYSQL_STATS_TYPES.has(dbType)) {
    const systemDbs = sqlInList(MYSQL_SYSTEM_DATABASES);
    const dbFilter = explicitDatabase
      ? `TABLE_SCHEMA = ${sqlLiteral(explicitDatabase)}`
      : `TABLE_SCHEMA NOT IN (${systemDbs})`;
    const scopeColumn = explicitDatabase ? "" : "TABLE_SCHEMA AS database_name, ";
    return `SELECT ${scopeColumn}TABLE_NAME AS table_name, INDEX_NAME AS index_name, GROUP_CONCAT(COLUMN_NAME ORDER BY SEQ_IN_INDEX SEPARATOR ', ') AS columns, CASE WHEN NON_UNIQUE = 0 THEN 'yes' ELSE 'no' END AS unique_index, INDEX_TYPE AS index_type FROM information_schema.STATISTICS WHERE ${dbFilter} GROUP BY TABLE_SCHEMA, TABLE_NAME, INDEX_NAME, NON_UNIQUE, INDEX_TYPE ORDER BY TABLE_SCHEMA, TABLE_NAME, INDEX_NAME`;
  }
  if (POSTGRES_STATS_TYPES.has(dbType)) {
    const systemSchemas = sqlInList(POSTGRES_SYSTEM_SCHEMAS);
    if (explicitSchema) {
      const schemaLit = sqlLiteral(explicitSchema);
      return `SELECT tablename AS table_name, indexname AS index_name, indexdef AS definition, CASE WHEN indexdef ILIKE '%UNIQUE%' THEN 'yes' ELSE 'no' END AS unique_index FROM pg_indexes WHERE schemaname = ${schemaLit} ORDER BY tablename, indexname`;
    }
    return `SELECT schemaname AS schema_name, tablename AS table_name, indexname AS index_name, indexdef AS definition, CASE WHEN indexdef ILIKE '%UNIQUE%' THEN 'yes' ELSE 'no' END AS unique_index FROM pg_indexes WHERE schemaname NOT IN (${systemSchemas}) ORDER BY schemaname, tablename, indexname`;
  }
  if (SQLITE_STATS_TYPES.has(dbType)) {
    return `SELECT tbl_name AS table_name, name AS index_name, sql AS definition, CASE WHEN sql LIKE 'CREATE UNIQUE INDEX%' THEN 'yes' ELSE 'no' END AS unique_index FROM sqlite_master WHERE type = 'index' AND name NOT LIKE 'sqlite_%' ORDER BY tbl_name, name`;
  }
  if (UNSUPPORTED_STATS_TYPES.has(dbType)) return null;
  const schemaName = explicitSchema || defaultStatsSchema(dbType);
  if (schemaName) {
    return `SELECT table_name, index_name, GROUP_CONCAT(column_name ORDER BY seq_in_index) AS columns, CASE WHEN non_unique = 0 THEN 'yes' ELSE 'no' END AS unique_index, index_type FROM information_schema.statistics WHERE table_schema = ${sqlLiteral(schemaName)} GROUP BY table_name, index_name, non_unique, index_type ORDER BY table_name, index_name`;
  }
  return null;
}

export function buildCatalogColumnCommentsSql(dbType: string, scope: CatalogStatsScope = {}): string | null {
  const explicitDatabase = scope.database?.trim();
  const explicitSchema = scope.schema?.trim();

  if (MYSQL_STATS_TYPES.has(dbType)) {
    const systemDbs = sqlInList(MYSQL_SYSTEM_DATABASES);
    const dbFilter = explicitDatabase
      ? `TABLE_SCHEMA = ${sqlLiteral(explicitDatabase)}`
      : `TABLE_SCHEMA NOT IN (${systemDbs})`;
    const scopeColumn = explicitDatabase ? "" : "TABLE_SCHEMA AS database_name, ";
    return `SELECT ${scopeColumn}TABLE_NAME AS table_name, COLUMN_NAME AS column_name, COLUMN_TYPE AS column_type, COLUMN_COMMENT AS comment FROM information_schema.COLUMNS WHERE ${dbFilter} AND COLUMN_COMMENT IS NOT NULL AND COLUMN_COMMENT != '' ORDER BY TABLE_SCHEMA, TABLE_NAME, ORDINAL_POSITION`;
  }
  if (POSTGRES_STATS_TYPES.has(dbType)) {
    const systemSchemas = sqlInList(POSTGRES_SYSTEM_SCHEMAS);
    if (explicitSchema) {
      const schemaLit = sqlLiteral(explicitSchema);
      return `SELECT c.table_name, c.column_name, c.data_type AS column_type, pg_catalog.col_description(format('%I.%I', c.table_schema, c.table_name)::regclass::oid, c.ordinal_position) AS comment FROM information_schema.columns c WHERE c.table_schema = ${schemaLit} AND pg_catalog.col_description(format('%I.%I', c.table_schema, c.table_name)::regclass::oid, c.ordinal_position) IS NOT NULL ORDER BY c.table_name, c.ordinal_position`;
    }
    return `SELECT c.table_schema AS schema_name, c.table_name, c.column_name, c.data_type AS column_type, pg_catalog.col_description(format('%I.%I', c.table_schema, c.table_name)::regclass::oid, c.ordinal_position) AS comment FROM information_schema.columns c WHERE c.table_schema NOT IN (${systemSchemas}) AND pg_catalog.col_description(format('%I.%I', c.table_schema, c.table_name)::regclass::oid, c.ordinal_position) IS NOT NULL ORDER BY c.table_schema, c.table_name, c.ordinal_position`;
  }
  if (SQLITE_STATS_TYPES.has(dbType)) return null;
  if (UNSUPPORTED_STATS_TYPES.has(dbType)) return null;
  const schemaName = explicitSchema || defaultStatsSchema(dbType);
  if (schemaName) {
    return `SELECT table_name, column_name, data_type AS column_type, '' AS comment FROM information_schema.columns WHERE table_schema = ${sqlLiteral(schemaName)} ORDER BY table_name, ordinal_position`;
  }
  return null;
}

function formatIndexTable(result: QueryResult, dbType: string): string {
  const isPostgres = POSTGRES_STATS_TYPES.has(dbType);
  const isSqlite = SQLITE_STATS_TYPES.has(dbType);
  const hasDatabase = result.rows.some((row) => row.database_name != null && String(row.database_name).trim() !== "");
  const hasSchema = !hasDatabase && result.rows.some((row) => row.schema_name != null && String(row.schema_name).trim() !== "");

  const rows = result.rows.map((row) => {
    if (isPostgres || isSqlite) {
      const cells = [
        formatCell(row.table_name),
        formatCell(row.index_name),
        formatCell(row.unique_index),
        formatCell(row.definition),
      ];
      if (hasSchema) return [formatCell(row.schema_name), ...cells];
      return cells;
    }
    const cells = [
      formatCell(row.table_name),
      formatCell(row.index_name),
      formatCell(row.columns),
      formatCell(row.unique_index),
      formatCell(row.index_type),
    ];
    if (hasDatabase) return [formatCell(row.database_name), ...cells];
    return cells;
  });

  if (isPostgres || isSqlite) {
    const headers = hasSchema
      ? ["Schema", "Table", "Index", "Unique", "Definition"]
      : ["Table", "Index", "Unique", "Definition"];
    return mdTable(headers, rows);
  }
  const headers = hasDatabase
    ? ["Database", "Table", "Index", "Columns", "Unique", "Type"]
    : ["Table", "Index", "Columns", "Unique", "Type"];
  return mdTable(headers, rows);
}

function formatColumnCommentsTable(result: QueryResult): string {
  const hasDatabase = result.rows.some((row) => row.database_name != null && String(row.database_name).trim() !== "");
  const hasSchema = !hasDatabase && result.rows.some((row) => row.schema_name != null && String(row.schema_name).trim() !== "");

  const rows = result.rows.map((row) => {
    const cells = [
      formatCell(row.table_name),
      formatCell(row.column_name),
      formatCell(row.column_type),
      formatCell(row.comment),
    ];
    if (hasDatabase) return [formatCell(row.database_name), ...cells];
    if (hasSchema) return [formatCell(row.schema_name), ...cells];
    return cells;
  });

  const headers = hasDatabase
    ? ["Database", "Table", "Column", "Type", "Comment"]
    : hasSchema
      ? ["Schema", "Table", "Column", "Type", "Comment"]
      : ["Table", "Column", "Type", "Comment"];
  return mdTable(headers, rows);
}

export async function fetchDatabaseReport(backend: Backend, config: ConnectionConfig, options: DatabaseStatsOptions = {}): Promise<string> {
  return withConnectionStage("Generating database report", async () => {
    const scopeValue = metadataScope(config, options.database, options.schema);
    const dbType = scopeValue.config.db_type;

  if (dbType === "redis" || dbType === "mongodb") {
    const statsBody = await fetchDatabaseStats(backend, scopeValue.config, options);
    return ["# Database Report", "", statsBody].join("\n");
  }
  if (isNonCatalogStatsType(dbType)) {
    throw new DatabaseStatsError("UNSUPPORTED_DB_TYPE", unsupportedStatsOverviewMessage(dbType, "report"));
  }

  const catalogScope = resolveCatalogStatsScope(dbType, options, scopeValue);
  const statsSql = buildCatalogStatsSql(dbType, catalogScope);
  if (!statsSql) {
    throw new DatabaseStatsError("UNSUPPORTED_DB_TYPE", unsupportedStatsOverviewMessage(dbType, "report"));
  }

  const parts: string[] = ["# Database Report", ""];
  const queryOptions = options.timeoutMs !== undefined ? { timeoutMs: options.timeoutMs } : undefined;

  let summaryText = "";
  const summarySql = buildCatalogSummarySql(dbType, catalogScope);
  if (summarySql) {
    try {
      const summary = await backend.executeQuery(scopeValue.config, summarySql, queryOptions);
      summaryText = formatSummaryLines(summary);
    } catch {
      // Summary is optional.
    }
  }

  const stats = await backend.executeQuery(scopeValue.config, statsSql, queryOptions);
  if (!summaryText) {
    summaryText = deriveCatalogSummaryFromStats(dbType, catalogScope, stats, scopeValue.config);
  }
  if (summaryText) parts.push("## Database Summary", summaryText);
  if (stats.rows.length === 0) {
    parts.push("", "No tables found in catalog.");
  } else {
    parts.push("", `## Tables (${stats.row_count})`, "_Sorted by estimated rows (descending)_", formatStatsOverviewTable(stats));
  }

  const columnCommentsSql = buildCatalogColumnCommentsSql(dbType, catalogScope);
  if (columnCommentsSql) {
    try {
      const columnComments = await backend.executeQuery(scopeValue.config, columnCommentsSql, queryOptions);
      if (columnComments.rows.length > 0) {
        parts.push("", `## Column Comments (${columnComments.row_count})`, formatColumnCommentsTable(columnComments));
      }
    } catch {
      // Column comments are optional.
    }
  }

  const indexSql = buildCatalogIndexSql(dbType, catalogScope);
  if (indexSql) {
    try {
      const indexes = await backend.executeQuery(scopeValue.config, indexSql, queryOptions);
      if (indexes.rows.length > 0) {
        parts.push("", `## Indexes (${indexes.row_count})`, formatIndexTable(indexes, dbType));
      } else {
        parts.push("", "## Indexes", "No indexes found in catalog.");
      }
    } catch {
      parts.push("", "## Indexes", "Index catalog unavailable for this database type.");
    }
  }

  return parts.join("\n");
  });
}

export type ReportSaveExtension = "md" | "json";

export interface ReportSavePathOptions {
  connectionName: string;
  database?: string;
  schema?: string;
  extension: ReportSaveExtension;
  timestamp?: string;
}

export function sanitizeReportFilenamePart(value: string): string {
  const sanitized = value
    .trim()
    .replace(/[^\w.-]+/g, "-")
    .replace(/^-+|-+$/g, "")
    .slice(0, 80);
  return sanitized || "default";
}

export function reportTimestamp(date = new Date()): string {
  const pad = (part: number) => String(part).padStart(2, "0");
  return `${date.getFullYear()}${pad(date.getMonth() + 1)}${pad(date.getDate())}-${pad(date.getHours())}${pad(date.getMinutes())}${pad(date.getSeconds())}`;
}

export function defaultReportsDir(): string {
  return join(process.cwd(), "reports");
}

export function reportScopeLabel(database?: string, schema?: string): string {
  return database?.trim() || schema?.trim() || "default";
}

export function buildReportSavePath(options: ReportSavePathOptions): string {
  const timestamp = options.timestamp ?? reportTimestamp();
  const scope = sanitizeReportFilenamePart(reportScopeLabel(options.database, options.schema));
  const filename = `dbx-report-${sanitizeReportFilenamePart(options.connectionName)}-${scope}-${timestamp}.${options.extension}`;
  return join(defaultReportsDir(), filename);
}

export function buildBatchReportDir(timestamp?: string): string {
  return join(defaultReportsDir(), `dbx-report-batch-${timestamp ?? reportTimestamp()}`);
}

export function buildBatchReportSavePath(
  dir: string,
  options: Omit<ReportSavePathOptions, "timestamp">,
): string {
  const scope = sanitizeReportFilenamePart(reportScopeLabel(options.database, options.schema));
  const filename = `dbx-report-${sanitizeReportFilenamePart(options.connectionName)}-${scope}.${options.extension}`;
  return join(dir, filename);
}
