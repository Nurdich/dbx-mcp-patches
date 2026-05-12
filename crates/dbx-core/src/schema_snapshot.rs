use chrono::{DateTime, Utc};
use futures::{stream, StreamExt, TryStreamExt};
use serde::{Deserialize, Serialize};

use crate::connection::AppState;
use crate::models::connection::{ConnectionConfig, DatabaseType};
use crate::{schema, types};

const TABLE_METADATA_CONCURRENCY: usize = 4;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TableSnapshot {
    pub name: String,
    pub table_type: String,
    pub comment: Option<String>,
    pub columns: Vec<types::ColumnInfo>,
    pub indexes: Vec<types::IndexInfo>,
    pub foreign_keys: Vec<types::ForeignKeyInfo>,
    pub triggers: Vec<types::TriggerInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SchemaSnapshot {
    pub connection_id: String,
    pub connection_name: String,
    pub database: Option<String>,
    pub database_type: DatabaseType,
    pub driver_profile: Option<String>,
    pub captured_at: DateTime<Utc>,
    pub databases: Vec<types::DatabaseInfo>,
    pub schemas: Vec<String>,
    pub tables: Vec<TableSnapshot>,
}

pub async fn snapshot(
    state: &AppState,
    connection_id: &str,
    database: Option<&str>,
    schema_name: Option<&str>,
) -> Result<SchemaSnapshot, String> {
    let config = {
        let configs = state.configs.read().await;
        configs.get(connection_id).cloned().ok_or("Connection config not found")?
    };

    let db = snapshot_database(&config, database)?;
    let databases = schema::list_databases_core(state, connection_id)
        .await
        .map_err(|err| format!("Failed to list databases: {err}"))?;
    let schemas = if db.is_empty() {
        Vec::new()
    } else {
        schema::list_schemas_core(state, connection_id, &db)
            .await
            .map_err(|err| format!("Failed to list schemas for database '{db}': {err}"))?
    };
    let effective_schema = schema_name.or_else(|| schemas.first().map(String::as_str)).unwrap_or("");
    let table_infos = if db.is_empty() {
        Vec::new()
    } else {
        schema::list_tables_core(state, connection_id, &db, effective_schema)
            .await
            .map_err(|err| format!("Failed to list tables for database '{db}' schema '{effective_schema}': {err}"))?
    };

    let tables = collect_table_snapshots(state, connection_id, &db, effective_schema, table_infos).await?;

    Ok(SchemaSnapshot {
        connection_id: config.id,
        connection_name: config.name,
        database: (!db.is_empty()).then_some(db),
        database_type: config.db_type,
        driver_profile: config.driver_profile,
        captured_at: Utc::now(),
        databases,
        schemas,
        tables,
    })
}

async fn collect_table_snapshots(
    state: &AppState,
    connection_id: &str,
    database: &str,
    schema_name: &str,
    table_infos: Vec<types::TableInfo>,
) -> Result<Vec<TableSnapshot>, String> {
    stream::iter(table_infos)
        .map(|table| async move { table_snapshot(state, connection_id, database, schema_name, table).await })
        .buffered(TABLE_METADATA_CONCURRENCY)
        .try_collect()
        .await
}

async fn table_snapshot(
    state: &AppState,
    connection_id: &str,
    database: &str,
    schema_name: &str,
    table: types::TableInfo,
) -> Result<TableSnapshot, String> {
    let table_name = table.name.clone();
    let columns = schema::get_columns_core(state, connection_id, database, schema_name, &table_name)
        .await
        .map_err(|err| format!("Failed to list columns for table '{table_name}': {err}"))?;
    let indexes = schema::list_indexes_core(state, connection_id, database, schema_name, &table_name)
        .await
        .map_err(|err| format!("Failed to list indexes for table '{table_name}': {err}"))?;
    let foreign_keys = schema::list_foreign_keys_core(state, connection_id, database, schema_name, &table_name)
        .await
        .map_err(|err| format!("Failed to list foreign keys for table '{table_name}': {err}"))?;
    let triggers = schema::list_triggers_core(state, connection_id, database, schema_name, &table_name)
        .await
        .map_err(|err| format!("Failed to list triggers for table '{table_name}': {err}"))?;

    Ok(TableSnapshot {
        name: table.name,
        table_type: table.table_type,
        comment: table.comment,
        columns,
        indexes,
        foreign_keys,
        triggers,
    })
}

fn snapshot_database(config: &ConnectionConfig, requested_database: Option<&str>) -> Result<String, String> {
    let database = requested_database
        .map(str::trim)
        .filter(|database| !database.is_empty())
        .or_else(|| config.effective_database())
        .or_else(|| embedded_default_database(&config.db_type))
        .map(str::to_string);

    match database {
        Some(database) => Ok(database),
        None if requires_database(&config.db_type) => Err(format!(
            "Database is required for schema snapshot for connection '{}' ({:?})",
            config.id, config.db_type
        )),
        None => Ok(String::new()),
    }
}

fn embedded_default_database(db_type: &DatabaseType) -> Option<&'static str> {
    match db_type {
        DatabaseType::Sqlite | DatabaseType::DuckDb => Some("main"),
        _ => None,
    }
}

fn requires_database(db_type: &DatabaseType) -> bool {
    matches!(
        db_type,
        DatabaseType::Mysql
            | DatabaseType::Doris
            | DatabaseType::StarRocks
            | DatabaseType::ClickHouse
            | DatabaseType::MongoDb
            | DatabaseType::Jdbc
    )
}
