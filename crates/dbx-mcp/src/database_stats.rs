//! Catalog-based database stats (TABLE_ROWS / n_live_tup — no COUNT(*)).

use std::collections::{HashMap, HashSet};

use dbx_core::models::connection::{ConnectionConfig, DatabaseType};
use dbx_core::types::QueryResult;
use serde_json::Value;

use crate::backend::DbxBackend;

const MYSQL_STATS_TYPES: &[&str] = &["mysql", "doris", "starrocks", "manticoresearch"];
const POSTGRES_STATS_TYPES: &[&str] = &[
    "postgres", "redshift", "gaussdb", "kwdb", "opengauss", "questdb", "kingbase", "highgo", "vastbase", "dameng",
];
const SQLITE_STATS_TYPES: &[&str] = &["sqlite", "rqlite"];
const NON_CATALOG_STATS_TYPES: &[&str] = &[
    "elasticsearch", "etcd", "neo4j", "cassandra", "milvus", "qdrant", "weaviate", "chromadb", "zookeeper", "mq",
    "kafka", "influxdb",
];

#[derive(Debug, Clone, Default)]
pub struct DatabaseStatsOptions {
    pub database: Option<String>,
    pub schema: Option<String>,
    pub redis_db: Option<u32>,
    pub timeout_ms: Option<u64>,
}

#[derive(Debug, Clone, Default)]
pub struct CatalogStatsScope {
    pub database: Option<String>,
    pub schema: Option<String>,
}

#[derive(Debug)]
pub struct DatabaseStatsError {
    pub code: &'static str,
    pub message: String,
}

impl std::fmt::Display for DatabaseStatsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for DatabaseStatsError {}

pub fn db_type_key(db_type: DatabaseType) -> String {
    serde_json::to_value(db_type)
        .ok()
        .and_then(|value| value.as_str().map(ToOwned::to_owned))
        .unwrap_or_else(|| format!("{:?}", db_type).to_ascii_lowercase())
}

pub fn is_non_catalog_stats_type(db_type: &str) -> bool {
    NON_CATALOG_STATS_TYPES.iter().any(|item| *item == db_type)
}

pub fn unsupported_stats_overview_message(db_type: &str, kind: &str) -> String {
    let noun = if kind == "report" { "report" } else { "stats overview" };
    format!(
        "Database {noun} is not supported for {db_type}. Supported: Redis, MongoDB, MySQL/MariaDB family, PostgreSQL family, SQLite/rqlite, and other SQL engines with information_schema."
    )
}

fn sql_literal(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

fn sql_in_list(values: &[&str]) -> String {
    values.iter().map(|value| sql_literal(value)).collect::<Vec<_>>().join(", ")
}

fn is_mysql(db_type: &str) -> bool {
    MYSQL_STATS_TYPES.iter().any(|item| *item == db_type)
}

fn is_postgres(db_type: &str) -> bool {
    POSTGRES_STATS_TYPES.iter().any(|item| *item == db_type)
}

fn is_sqlite(db_type: &str) -> bool {
    SQLITE_STATS_TYPES.iter().any(|item| *item == db_type)
}

pub fn resolve_catalog_stats_scope(
    db_type: &str,
    options: &DatabaseStatsOptions,
    config: &ConnectionConfig,
    schema_hint: Option<&str>,
) -> CatalogStatsScope {
    let explicit_database = options.database.as_deref().map(str::trim).filter(|v| !v.is_empty()).map(str::to_string);
    let explicit_schema = options.schema.as_deref().map(str::trim).filter(|v| !v.is_empty()).map(str::to_string);
    let config_database = config.database.as_deref().map(str::trim).filter(|v| !v.is_empty()).map(str::to_string);

    if db_type == "dameng" {
        return CatalogStatsScope {
            database: explicit_database.clone().or(config_database.clone()),
            schema: explicit_schema
                .or(explicit_database)
                .or_else(|| schema_hint.map(str::to_string))
                .or(config_database),
        };
    }
    if is_mysql(db_type) {
        return CatalogStatsScope { database: explicit_database.or(config_database), schema: explicit_schema };
    }
    CatalogStatsScope { database: explicit_database.or(config_database), schema: explicit_schema }
}

pub fn build_catalog_stats_sql(db_type: &str, scope: &CatalogStatsScope) -> Option<String> {
    let explicit_database = scope.database.as_deref().map(str::trim).filter(|v| !v.is_empty());
    let explicit_schema = scope.schema.as_deref().map(str::trim).filter(|v| !v.is_empty());
    let mysql_system = ["information_schema", "mysql", "performance_schema", "sys"];
    let postgres_system = ["information_schema", "pg_catalog", "pg_toast"];
    let generic_system = ["information_schema", "pg_catalog", "mysql", "performance_schema", "sys"];

    if is_mysql(db_type) {
        let db_filter = if let Some(db) = explicit_database {
            format!("TABLE_SCHEMA = {}", sql_literal(db))
        } else {
            format!("TABLE_SCHEMA NOT IN ({})", sql_in_list(&mysql_system))
        };
        let scope_column = if explicit_database.is_some() { "" } else { "TABLE_SCHEMA AS database_name, " };
        let table_type_filter = if explicit_database.is_some() {
            "TABLE_TYPE = 'BASE TABLE'"
        } else {
            "TABLE_TYPE IN ('BASE TABLE', 'VIEW')"
        };
        return Some(format!(
            "SELECT {scope_column}TABLE_NAME AS name, TABLE_TYPE AS type, ENGINE AS engine, TABLE_ROWS AS rows_estimate, DATA_LENGTH AS data_bytes, INDEX_LENGTH AS index_bytes, (COALESCE(DATA_LENGTH, 0) + COALESCE(INDEX_LENGTH, 0)) AS total_bytes, TABLE_COMMENT AS comment FROM information_schema.TABLES WHERE {db_filter} AND {table_type_filter}"
        ));
    }
    if is_postgres(db_type) {
        let system = sql_in_list(&postgres_system);
        if let Some(schema) = explicit_schema {
            let schema_lit = sql_literal(schema);
            return Some(format!(
                "SELECT t.table_name AS name, t.table_type AS type, NULL AS engine, COALESCE(st.n_live_tup, FLOOR(c.reltuples))::bigint AS rows_estimate, pg_relation_size(c.oid) AS data_bytes, GREATEST(pg_total_relation_size(c.oid) - pg_relation_size(c.oid), 0) AS index_bytes, pg_total_relation_size(c.oid) AS total_bytes, obj_description(c.oid, 'pg_class') AS comment FROM information_schema.tables t INNER JOIN pg_catalog.pg_class c ON c.relname = t.table_name INNER JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace AND n.nspname = t.table_schema LEFT JOIN pg_catalog.pg_stat_user_tables st ON st.schemaname = t.table_schema AND st.relname = t.table_name WHERE t.table_schema = {schema_lit} AND t.table_type IN ('BASE TABLE', 'VIEW')"
            ));
        }
        return Some(format!(
            "SELECT t.table_schema AS schema_name, t.table_name AS name, t.table_type AS type, NULL AS engine, COALESCE(st.n_live_tup, FLOOR(c.reltuples))::bigint AS rows_estimate, pg_relation_size(c.oid) AS data_bytes, GREATEST(pg_total_relation_size(c.oid) - pg_relation_size(c.oid), 0) AS index_bytes, pg_total_relation_size(c.oid) AS total_bytes, obj_description(c.oid, 'pg_class') AS comment FROM information_schema.tables t INNER JOIN pg_catalog.pg_class c ON c.relname = t.table_name INNER JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace AND n.nspname = t.table_schema LEFT JOIN pg_catalog.pg_stat_user_tables st ON st.schemaname = t.table_schema AND st.relname = t.table_name WHERE t.table_schema NOT IN ({system}) AND t.table_type IN ('BASE TABLE', 'VIEW')"
        ));
    }
    if is_sqlite(db_type) {
        return Some(
            "SELECT name, type, NULL AS engine, NULL AS rows_estimate, NULL AS data_bytes, NULL AS index_bytes, NULL AS total_bytes, NULL AS comment FROM sqlite_master WHERE type IN ('table', 'view') AND name NOT LIKE 'sqlite_%'"
                .to_string(),
        );
    }
    if db_type == "redis" || db_type == "mongodb" || is_non_catalog_stats_type(db_type) {
        return None;
    }
    let schema_name = explicit_schema.unwrap_or(if db_type == "sqlserver" { "dbo" } else { "public" });
    if !schema_name.is_empty() {
        return Some(format!(
            "SELECT table_name AS name, table_type AS type, NULL AS engine, NULL AS rows_estimate, NULL AS data_bytes, NULL AS index_bytes, NULL AS total_bytes, NULL AS comment FROM information_schema.tables WHERE table_schema = {} AND table_type IN ('BASE TABLE', 'VIEW')",
            sql_literal(schema_name)
        ));
    }
    Some(format!(
        "SELECT table_name AS name, table_type AS type, NULL AS engine, NULL AS rows_estimate, NULL AS data_bytes, NULL AS index_bytes, NULL AS total_bytes, NULL AS comment FROM information_schema.tables WHERE table_schema NOT IN ({}) AND table_type IN ('BASE TABLE', 'VIEW')",
        sql_in_list(&generic_system)
    ))
}

fn row_maps(result: &QueryResult) -> Vec<HashMap<String, Value>> {
    result
        .rows
        .iter()
        .map(|row| {
            result
                .columns
                .iter()
                .enumerate()
                .filter_map(|(idx, column)| row.get(idx).cloned().map(|value| (column.clone(), value)))
                .collect()
        })
        .collect()
}

fn parse_row_count(row: &HashMap<String, Value>) -> Option<f64> {
    for field in ["rows_estimate", "TABLE_ROWS", "n_live_tup", "reltuples", "count", "nrecords", "Rows", "Docs"] {
        let Some(value) = row.get(field) else { continue };
        if value.is_null() {
            continue;
        }
        let n = value.as_f64().or_else(|| value.as_i64().map(|v| v as f64)).or_else(|| {
            value.as_str().and_then(|s| s.parse().ok())
        })?;
        if n.is_finite() && n >= 0.0 {
            return Some(n);
        }
    }
    None
}

fn sort_stats_rows(mut rows: Vec<HashMap<String, Value>>) -> Vec<HashMap<String, Value>> {
    rows.sort_by(|a, b| {
        let a_count = parse_row_count(a);
        let b_count = parse_row_count(b);
        match (a_count, b_count) {
            (None, None) => std::cmp::Ordering::Equal,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (Some(_), None) => std::cmp::Ordering::Less,
            (Some(a), Some(b)) => b.partial_cmp(&a).unwrap_or(std::cmp::Ordering::Equal),
        }
    });
    rows
}

fn format_cell(value: &Value) -> String {
    match value {
        Value::Null => String::new(),
        Value::String(text) => text.replace('|', "\\|").replace('\n', " "),
        other => other.to_string().trim_matches('"').replace('|', "\\|"),
    }
}

fn format_stat_bytes(value: &Value) -> String {
    let Some(n) = value.as_f64().or_else(|| value.as_i64().map(|v| v as f64)) else {
        return String::new();
    };
    if !n.is_finite() || n < 0.0 {
        return String::new();
    }
    if n == 0.0 {
        return "0 B".to_string();
    }
    let units = ["B", "KB", "MB", "GB", "TB"];
    let mut size = n;
    let mut unit = 0usize;
    while size >= 1024.0 && unit + 1 < units.len() {
        size /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{} {}", size.round() as i64, units[unit])
    } else if size < 10.0 {
        format!("{size:.1} {}", units[unit])
    } else {
        format!("{} {}", size.round() as i64, units[unit])
    }
}

fn markdown_table(headers: &[&str], rows: &[Vec<String>]) -> String {
    let mut output = format!("| {} |\n| {} |", headers.join(" | "), vec!["---"; headers.len()].join(" | "));
    for row in rows {
        output.push_str(&format!("\n| {} |", row.join(" | ")));
    }
    output
}

pub fn format_stats_overview_table(result: &QueryResult) -> String {
    let sorted = sort_stats_rows(row_maps(result));
    let has_database = sorted.iter().any(|row| {
        row.get("database_name").is_some_and(|value| !format_cell(value).trim().is_empty())
    });
    let has_schema = !has_database
        && sorted.iter().any(|row| row.get("schema_name").is_some_and(|value| !format_cell(value).trim().is_empty()));
    let headers: &[&str] = if has_database {
        &["Database", "Name", "Type", "Engine", "Rows (est.)", "Data", "Index", "Total", "Comment"]
    } else if has_schema {
        &["Schema", "Name", "Type", "Engine", "Rows (est.)", "Data", "Index", "Total", "Comment"]
    } else {
        &["Name", "Type", "Engine", "Rows (est.)", "Data", "Index", "Total", "Comment"]
    };
    let rows: Vec<Vec<String>> = sorted
        .iter()
        .map(|row| {
            let mut cells = vec![
                format_cell(row.get("name").unwrap_or(&Value::Null)),
                format_cell(row.get("type").unwrap_or(&Value::Null)),
                format_cell(row.get("engine").unwrap_or(&Value::Null)),
                format_cell(row.get("rows_estimate").unwrap_or(&Value::Null)),
                format_stat_bytes(row.get("data_bytes").unwrap_or(&Value::Null)),
                format_stat_bytes(row.get("index_bytes").unwrap_or(&Value::Null)),
                format_stat_bytes(row.get("total_bytes").unwrap_or(&Value::Null)),
                format_cell(row.get("comment").unwrap_or(&Value::Null)),
            ];
            if has_database {
                cells.insert(0, format_cell(row.get("database_name").unwrap_or(&Value::Null)));
            } else if has_schema {
                cells.insert(0, format_cell(row.get("schema_name").unwrap_or(&Value::Null)));
            }
            cells
        })
        .collect();
    markdown_table(headers, &rows)
}

fn unique_non_empty_field_count(rows: &[HashMap<String, Value>], field: &str) -> usize {
    let mut seen = HashSet::new();
    for row in rows {
        if let Some(value) = row.get(field) {
            let text = format_cell(value);
            if !text.trim().is_empty() {
                seen.insert(text);
            }
        }
    }
    seen.len()
}

fn derive_catalog_summary(db_type: &str, scope: &CatalogStatsScope, stats: &QueryResult, config: &ConnectionConfig) -> String {
    let rows = row_maps(stats);
    let table_count = rows.len();
    if is_mysql(db_type) {
        if let Some(db) = scope.database.as_deref() {
            return format!("database_name: {db}\ntable_count: {table_count}");
        }
        return format!(
            "database_count: {}\ntable_count: {table_count}",
            unique_non_empty_field_count(&rows, "database_name")
        );
    }
    if is_postgres(db_type) && scope.schema.is_none() {
        return format!(
            "database_name: {}\nschema_count: {}\ntable_count: {table_count}",
            config.database.clone().unwrap_or_default(),
            unique_non_empty_field_count(&rows, "schema_name")
        );
    }
    if is_sqlite(db_type) {
        return format!("database_name: main\nobject_count: {table_count}");
    }
    String::new()
}

async fn fetch_redis_stats(
    backend: &dyn DbxBackend,
    config: &ConnectionConfig,
    options: &DatabaseStatsOptions,
) -> Result<String, DatabaseStatsError> {
    let redis_db = options.redis_db.or_else(|| {
        config.database.as_deref().and_then(|value| value.trim().parse().ok())
    }).unwrap_or(0);
    let info = backend
        .execute_redis_command(config, redis_db, "INFO", false)
        .await
        .map_err(|message| DatabaseStatsError { code: "REDIS_STATS_ERROR", message })?;
    let dbsize = backend
        .execute_redis_command(config, redis_db, "DBSIZE", false)
        .await
        .map_err(|message| DatabaseStatsError { code: "REDIS_STATS_ERROR", message })?;
    let info_text = match &info.value {
        Value::String(text) => text.clone(),
        other => other.to_string(),
    };
    let mut sections: HashMap<String, HashMap<String, String>> = HashMap::new();
    let mut section = "server".to_string();
    for line in info_text.lines() {
        if line.is_empty() || line.starts_with('#') {
            let title = line.trim_start_matches('#').trim().to_ascii_lowercase().replace(' ', "_");
            if !title.is_empty() {
                section = title;
            }
            continue;
        }
        if let Some((key, value)) = line.split_once(':') {
            sections.entry(section.clone()).or_default().insert(key.trim().to_string(), value.trim().to_string());
        }
    }
    let memory = sections.get("memory").cloned().unwrap_or_default();
    let keyspace = sections.get("keyspace").cloned().unwrap_or_default();
    let server = sections.get("server").cloned().unwrap_or_default();
    let clients = sections.get("clients").cloned().unwrap_or_default();
    let summary_rows = vec![
        vec!["redis_version".into(), server.get("redis_version").cloned().unwrap_or_default()],
        vec!["role".into(), server.get("role").cloned().unwrap_or_default()],
        vec!["used_memory_human".into(), memory.get("used_memory_human").cloned().unwrap_or_default()],
        vec!["used_memory_peak_human".into(), memory.get("used_memory_peak_human").cloned().unwrap_or_default()],
        vec!["connected_clients".into(), clients.get("connected_clients").cloned().unwrap_or_default()],
        vec!["db".into(), redis_db.to_string()],
        vec!["dbsize".into(), format_cell(&dbsize.value)],
    ];
    let mut parts = vec!["Summary".to_string(), markdown_table(&["Metric", "Value"], &summary_rows)];
    if !keyspace.is_empty() {
        let keyspace_rows: Vec<Vec<String>> = keyspace.into_iter().map(|(k, v)| vec![k, v]).collect();
        parts.push(String::new());
        parts.push("Keyspace".to_string());
        parts.push(markdown_table(&["DB", "Stats"], &keyspace_rows));
    }
    Ok(parts.join("\n"))
}

pub async fn fetch_database_stats(
    backend: &dyn DbxBackend,
    config: &ConnectionConfig,
    options: DatabaseStatsOptions,
) -> Result<String, DatabaseStatsError> {
    let db_type = db_type_key(config.db_type);
    if db_type == "redis" {
        return fetch_redis_stats(backend, config, &options).await;
    }
    if db_type == "mongodb" {
        let database = options.database.clone().or_else(|| config.database.clone()).unwrap_or_default();
        let tables = backend
            .list_tables(config, &database, options.schema.as_deref().unwrap_or(""))
            .await
            .map_err(|message| DatabaseStatsError { code: "MONGO_STATS_ERROR", message })?;
        if tables.is_empty() {
            return Ok("No collections found.".to_string());
        }
        let limit = tables.len().min(50);
        let mut rows = Vec::new();
        for table in tables.iter().take(limit) {
            match backend.execute_query(config, &database, &format!("db.{}.stats()", table.name), None, options.timeout_ms.map(|ms| (ms + 999) / 1000)).await {
                Ok(result) => {
                    let map = row_maps(&result).into_iter().next().unwrap_or_default();
                    rows.push(vec![
                        table.name.clone(),
                        table.table_type.clone(),
                        String::new(),
                        format_cell(map.get("count").or_else(|| map.get("nrecords")).unwrap_or(&Value::Null)),
                        format_stat_bytes(map.get("size").or_else(|| map.get("avgObjSize")).unwrap_or(&Value::Null)),
                        format_stat_bytes(map.get("totalIndexSize").unwrap_or(&Value::Null)),
                        format_stat_bytes(map.get("storageSize").unwrap_or(&Value::Null)),
                        format_cell(map.get("storageEngine").unwrap_or(&Value::Null)),
                    ]);
                }
                Err(_) => rows.push(vec![
                    table.name.clone(),
                    table.table_type.clone(),
                    String::new(),
                    String::new(),
                    String::new(),
                    String::new(),
                    String::new(),
                    "stats unavailable".into(),
                ]),
            }
        }
        let header = if tables.len() > limit {
            format!("\nShowing {limit} of {} collections (catalog stats).\n", tables.len())
        } else {
            String::new()
        };
        return Ok(format!(
            "{header}{}",
            markdown_table(
                &["Name", "Type", "Engine", "Docs (est.)", "Data", "Index", "Storage", "Storage Engine"],
                &rows
            )
        ));
    }
    if is_non_catalog_stats_type(&db_type) {
        return Err(DatabaseStatsError {
            code: "UNSUPPORTED_DB_TYPE",
            message: unsupported_stats_overview_message(&db_type, "stats"),
        });
    }

    let catalog_scope = resolve_catalog_stats_scope(&db_type, &options, config, options.schema.as_deref());
    let Some(stats_sql) = build_catalog_stats_sql(&db_type, &catalog_scope) else {
        return Err(DatabaseStatsError {
            code: "UNSUPPORTED_DB_TYPE",
            message: unsupported_stats_overview_message(&db_type, "stats"),
        });
    };

    let mut query_config = config.clone();
    if let Some(database) = catalog_scope.database.clone() {
        if db_type != "dameng" {
            query_config.database = Some(database.clone());
        }
    }
    let database = query_config.database.clone().unwrap_or_default();
    let timeout_secs = options.timeout_ms.map(|ms| (ms + 999) / 1000);
    let stats = backend
        .execute_query(&query_config, &database, &stats_sql, None, timeout_secs)
        .await
        .map_err(|message| DatabaseStatsError { code: "STATS_QUERY_ERROR", message })?;

    let mut parts = Vec::new();
    let summary = derive_catalog_summary(&db_type, &catalog_scope, &stats, &query_config);
    if !summary.is_empty() {
        parts.push("Summary".to_string());
        parts.push(summary);
    }
    if stats.rows.is_empty() {
        if !parts.is_empty() {
            parts.push(String::new());
        }
        parts.push("No tables found in catalog.".to_string());
    } else {
        if !parts.is_empty() {
            parts.push(String::new());
        }
        let all_scopes = if is_mysql(&db_type) {
            catalog_scope.database.is_none()
        } else if is_postgres(&db_type) {
            catalog_scope.schema.is_none()
        } else {
            catalog_scope.schema.is_none()
        };
        let scope_label = if all_scopes { " (all user scopes)" } else { "" };
        parts.push(format!("Tables ({}){scope_label}", stats.rows.len()));
        parts.push(format_stats_overview_table(&stats));
    }
    Ok(parts.join("\n"))
}
