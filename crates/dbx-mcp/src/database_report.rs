//! Full database report: summary + tables (rows desc) + column comments + indexes.

use dbx_core::models::connection::ConnectionConfig;
use dbx_core::types::QueryResult;

use crate::backend::DbxBackend;
use crate::database_stats::{
    build_catalog_stats_sql, db_type_key, fetch_database_stats, format_stats_overview_table,
    is_non_catalog_stats_type, resolve_catalog_stats_scope, unsupported_stats_overview_message,
    CatalogStatsScope, DatabaseStatsError, DatabaseStatsOptions,
};

fn sql_literal(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

fn sql_in_list(values: &[&str]) -> String {
    values.iter().map(|value| sql_literal(value)).collect::<Vec<_>>().join(", ")
}

fn is_mysql(db_type: &str) -> bool {
    matches!(db_type, "mysql" | "doris" | "starrocks" | "manticoresearch")
}

fn is_postgres(db_type: &str) -> bool {
    matches!(
        db_type,
        "postgres"
            | "redshift"
            | "gaussdb"
            | "kwdb"
            | "opengauss"
            | "questdb"
            | "kingbase"
            | "highgo"
            | "vastbase"
            | "dameng"
    )
}

fn is_sqlite(db_type: &str) -> bool {
    matches!(db_type, "sqlite" | "rqlite")
}

pub fn build_catalog_index_sql(db_type: &str, scope: &CatalogStatsScope) -> Option<String> {
    let explicit_database = scope.database.as_deref().map(str::trim).filter(|v| !v.is_empty());
    let explicit_schema = scope.schema.as_deref().map(str::trim).filter(|v| !v.is_empty());
    let mysql_system = ["information_schema", "mysql", "performance_schema", "sys"];
    let postgres_system = ["information_schema", "pg_catalog", "pg_toast"];

    if is_mysql(db_type) {
        let db_filter = if let Some(db) = explicit_database {
            format!("TABLE_SCHEMA = {}", sql_literal(db))
        } else {
            format!("TABLE_SCHEMA NOT IN ({})", sql_in_list(&mysql_system))
        };
        let scope_column = if explicit_database.is_some() { "" } else { "TABLE_SCHEMA AS database_name, " };
        return Some(format!(
            "SELECT {scope_column}TABLE_NAME AS table_name, INDEX_NAME AS index_name, GROUP_CONCAT(COLUMN_NAME ORDER BY SEQ_IN_INDEX SEPARATOR ', ') AS columns, CASE WHEN NON_UNIQUE = 0 THEN 'yes' ELSE 'no' END AS unique_index, INDEX_TYPE AS index_type FROM information_schema.STATISTICS WHERE {db_filter} GROUP BY TABLE_SCHEMA, TABLE_NAME, INDEX_NAME, NON_UNIQUE, INDEX_TYPE ORDER BY TABLE_SCHEMA, TABLE_NAME, INDEX_NAME"
        ));
    }
    if is_postgres(db_type) {
        let system = sql_in_list(&postgres_system);
        if let Some(schema) = explicit_schema {
            let schema_lit = sql_literal(schema);
            return Some(format!(
                "SELECT tablename AS table_name, indexname AS index_name, indexdef AS definition, CASE WHEN indexdef ILIKE '%UNIQUE%' THEN 'yes' ELSE 'no' END AS unique_index FROM pg_indexes WHERE schemaname = {schema_lit} ORDER BY tablename, indexname"
            ));
        }
        return Some(format!(
            "SELECT schemaname AS schema_name, tablename AS table_name, indexname AS index_name, indexdef AS definition, CASE WHEN indexdef ILIKE '%UNIQUE%' THEN 'yes' ELSE 'no' END AS unique_index FROM pg_indexes WHERE schemaname NOT IN ({system}) ORDER BY schemaname, tablename, indexname"
        ));
    }
    if is_sqlite(db_type) {
        return Some(
            "SELECT tbl_name AS table_name, name AS index_name, sql AS definition, CASE WHEN sql LIKE 'CREATE UNIQUE INDEX%' THEN 'yes' ELSE 'no' END AS unique_index FROM sqlite_master WHERE type = 'index' AND name NOT LIKE 'sqlite_%' ORDER BY tbl_name, name"
                .to_string(),
        );
    }
    None
}

pub fn build_catalog_column_comments_sql(db_type: &str, scope: &CatalogStatsScope) -> Option<String> {
    let explicit_database = scope.database.as_deref().map(str::trim).filter(|v| !v.is_empty());
    let explicit_schema = scope.schema.as_deref().map(str::trim).filter(|v| !v.is_empty());
    let mysql_system = ["information_schema", "mysql", "performance_schema", "sys"];
    let postgres_system = ["information_schema", "pg_catalog", "pg_toast"];

    if is_mysql(db_type) {
        let db_filter = if let Some(db) = explicit_database {
            format!("TABLE_SCHEMA = {}", sql_literal(db))
        } else {
            format!("TABLE_SCHEMA NOT IN ({})", sql_in_list(&mysql_system))
        };
        let scope_column = if explicit_database.is_some() { "" } else { "TABLE_SCHEMA AS database_name, " };
        return Some(format!(
            "SELECT {scope_column}TABLE_NAME AS table_name, COLUMN_NAME AS column_name, COLUMN_TYPE AS column_type, COLUMN_COMMENT AS comment FROM information_schema.COLUMNS WHERE {db_filter} AND COLUMN_COMMENT IS NOT NULL AND COLUMN_COMMENT != '' ORDER BY TABLE_SCHEMA, TABLE_NAME, ORDINAL_POSITION"
        ));
    }
    if is_postgres(db_type) {
        let system = sql_in_list(&postgres_system);
        if let Some(schema) = explicit_schema {
            let schema_lit = sql_literal(schema);
            return Some(format!(
                "SELECT c.table_name, c.column_name, c.data_type AS column_type, pg_catalog.col_description(format('%I.%I', c.table_schema, c.table_name)::regclass::oid, c.ordinal_position) AS comment FROM information_schema.columns c WHERE c.table_schema = {schema_lit} AND pg_catalog.col_description(format('%I.%I', c.table_schema, c.table_name)::regclass::oid, c.ordinal_position) IS NOT NULL ORDER BY c.table_name, c.ordinal_position"
            ));
        }
        return Some(format!(
            "SELECT c.table_schema AS schema_name, c.table_name, c.column_name, c.data_type AS column_type, pg_catalog.col_description(format('%I.%I', c.table_schema, c.table_name)::regclass::oid, c.ordinal_position) AS comment FROM information_schema.columns c WHERE c.table_schema NOT IN ({system}) AND pg_catalog.col_description(format('%I.%I', c.table_schema, c.table_name)::regclass::oid, c.ordinal_position) IS NOT NULL ORDER BY c.table_schema, c.table_name, c.ordinal_position"
        ));
    }
    None
}

fn format_query_as_markdown(title: &str, result: &QueryResult) -> String {
    if result.rows.is_empty() {
        return format!("## {title}\n\n(none)\n");
    }
    let headers: Vec<&str> = result.columns.iter().map(String::as_str).collect();
    let mut output = format!("## {title}\n\n| {} |\n| {} |", headers.join(" | "), vec!["---"; headers.len()].join(" | "));
    for row in &result.rows {
        let cells: Vec<String> = result
            .columns
            .iter()
            .enumerate()
            .map(|(idx, _)| {
                row.get(idx)
                    .map(|value| match value {
                        serde_json::Value::Null => String::new(),
                        serde_json::Value::String(text) => text.replace('|', "\\|").replace('\n', " "),
                        other => other.to_string().trim_matches('"').replace('|', "\\|"),
                    })
                    .unwrap_or_default()
            })
            .collect();
        output.push_str(&format!("\n| {} |", cells.join(" | ")));
    }
    output.push('\n');
    output
}

pub async fn fetch_database_report(
    backend: &dyn DbxBackend,
    config: &ConnectionConfig,
    options: DatabaseStatsOptions,
) -> Result<String, DatabaseStatsError> {
    let db_type = db_type_key(config.db_type);
    if db_type == "redis" || db_type == "mongodb" {
        let stats = fetch_database_stats(backend, config, options).await?;
        return Ok(format!("# Database report ({db_type})\n\n{stats}\n"));
    }
    if is_non_catalog_stats_type(&db_type) {
        return Err(DatabaseStatsError {
            code: "UNSUPPORTED_DB_TYPE",
            message: unsupported_stats_overview_message(&db_type, "report"),
        });
    }

    let catalog_scope = resolve_catalog_stats_scope(&db_type, &options, config, options.schema.as_deref());
    let Some(stats_sql) = build_catalog_stats_sql(&db_type, &catalog_scope) else {
        return Err(DatabaseStatsError {
            code: "UNSUPPORTED_DB_TYPE",
            message: unsupported_stats_overview_message(&db_type, "report"),
        });
    };

    let mut query_config = config.clone();
    if let Some(database) = catalog_scope.database.clone() {
        if db_type != "dameng" {
            query_config.database = Some(database);
        }
    }
    let database = query_config.database.clone().unwrap_or_default();
    let timeout_secs = options.timeout_ms.map(|ms| (ms + 999) / 1000);

    let stats = backend
        .execute_query(&query_config, &database, &stats_sql, None, timeout_secs)
        .await
        .map_err(|message| DatabaseStatsError { code: "REPORT_QUERY_ERROR", message })?;

    let mut parts = vec![
        format!("# Database report — {}", config.name),
        String::new(),
        format!("## Tables ({})", stats.rows.len()),
        format_stats_overview_table(&stats),
    ];

    if let Some(comments_sql) = build_catalog_column_comments_sql(&db_type, &catalog_scope) {
        match backend.execute_query(&query_config, &database, &comments_sql, None, timeout_secs).await {
            Ok(result) => parts.push(format_query_as_markdown("Column comments", &result)),
            Err(error) => parts.push(format!("## Column comments\n\n(unavailable: {error})\n")),
        }
    }

    if let Some(index_sql) = build_catalog_index_sql(&db_type, &catalog_scope) {
        match backend.execute_query(&query_config, &database, &index_sql, None, timeout_secs).await {
            Ok(result) => parts.push(format_query_as_markdown("Indexes", &result)),
            Err(error) => parts.push(format!("## Indexes\n\n(unavailable: {error})\n")),
        }
    }

    Ok(parts.join("\n"))
}

/// Default report directory: `{cwd}/reports`.
pub fn default_reports_dir() -> std::path::PathBuf {
    std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from(".")).join("reports")
}
