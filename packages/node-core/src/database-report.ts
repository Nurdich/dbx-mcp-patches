import type { Backend } from "./backend.js";
import type { ConnectionConfig } from "./connections.js";
import type { QueryResult } from "./database.js";
import { withConnectionStage } from "./connection-log.js";
import { formatCell, mdTable } from "./format.js";
import {
  DatabaseStatsError,
  buildCatalogStatsSql,
  buildCatalogSummarySql,
  deriveCatalogSummaryFromStats,
  fetchDatabaseStats,
  formatStatsOverviewTable,
  metadataScope,
  type CatalogStatsScope,
  type DatabaseStatsOptions,
} from "./database-stats.js";

const MYSQL_STATS_TYPES = new Set(["mysql", "doris", "starrocks", "manticoresearch"]);
const POSTGRES_STATS_TYPES = new Set(["postgres", "redshift", "gaussdb", "kwdb", "opengauss", "questdb", "kingbase", "highgo", "vastbase", "dameng"]);
const SQLITE_STATS_TYPES = new Set(["sqlite", "rqlite"]);
const UNSUPPORTED_STATS_TYPES = new Set(["redis", "mongodb", "elasticsearch", "etcd", "neo4j", "cassandra", "milvus", "qdrant", "weaviate", "chromadb", "zookeeper"]);

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

function resolveCatalogStatsScope(
  dbType: string,
  options: DatabaseStatsOptions,
  scopeValue: { schema?: string },
): CatalogStatsScope {
  const explicitDatabase = options.database?.trim();
  const explicitSchema = options.schema?.trim();
  if (dbType === "dameng") {
    return {
      database: explicitDatabase,
      schema: explicitSchema || explicitDatabase || scopeValue.schema,
    };
  }
  return {
    database: explicitDatabase,
    schema: explicitSchema,
  };
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

  const catalogScope = resolveCatalogStatsScope(dbType, options, scopeValue);
  const statsSql = buildCatalogStatsSql(dbType, catalogScope);
  if (!statsSql) {
    throw new DatabaseStatsError(
      "UNSUPPORTED_DB_TYPE",
      `Database report is not supported for ${dbType}. Supported: MySQL/MariaDB family, PostgreSQL family, SQLite/rqlite, and other SQL engines with information_schema.`,
    );
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
