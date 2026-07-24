//! Catalog database stats / report HTTP API (MCP/CLI parity).

use std::sync::Arc;

use async_trait::async_trait;
use axum::extract::State;
use axum::Json;
use dbx_core::connection::AppState;
use dbx_core::database_stats::{
    CatalogStatsExecutor, DatabaseStatsError, DatabaseStatsOptions,
};
use dbx_core::db::redis_driver::RedisCommandResult;
use dbx_core::models::connection::{ConnectionConfig, TransportLayerConfig};
use dbx_core::proxy_profiles::{
    apply_proxy_profiles_failover, has_proxy_profile_ref, resolve_proxy_profiles, ProxyProfileRefArgs,
};
use dbx_core::types::{QueryResult, TableInfo};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::AppError;
use crate::state::WebState;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DatabaseCatalogRequest {
    pub connection_id: Option<String>,
    pub connection_ids: Option<Vec<String>>,
    pub database: Option<String>,
    pub schema: Option<String>,
    pub redis_db: Option<u32>,
    pub timeout_ms: Option<u64>,
    #[serde(default = "default_skip_unsupported")]
    pub skip_unsupported: bool,
    pub proxy_profile_id: Option<String>,
    pub proxy_profile_name: Option<String>,
    pub proxy_profile_ids: Option<Vec<String>>,
    pub proxy_profile_names: Option<Vec<String>>,
}

fn default_skip_unsupported() -> bool {
    true
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DatabaseCatalogResponse {
    pub markdown: String,
    pub total: usize,
    pub success: usize,
    pub skipped: usize,
    pub failures: usize,
}

struct AppCatalogExecutor {
    app: Arc<AppState>,
}

#[async_trait]
impl CatalogStatsExecutor for AppCatalogExecutor {
    async fn execute_query(
        &self,
        connection: &ConnectionConfig,
        database: &str,
        sql: &str,
        timeout_secs: Option<u64>,
    ) -> Result<QueryResult, String> {
        ensure_config_ready(&self.app, connection).await?;
        dbx_core::query::execute_sql_statement_with_options(
            &self.app,
            &connection.id,
            database,
            sql,
            None,
            None,
            dbx_core::query::QueryExecutionOptions { timeout_secs, ..Default::default() },
        )
        .await
    }

    async fn execute_redis_command(
        &self,
        connection: &ConnectionConfig,
        database: u32,
        command: &str,
    ) -> Result<RedisCommandResult, String> {
        ensure_config_ready(&self.app, connection).await?;
        dbx_core::redis_ops::redis_execute_command_core(&self.app, &connection.id, database, command, false).await
    }

    async fn list_tables(
        &self,
        connection: &ConnectionConfig,
        database: &str,
        schema: &str,
    ) -> Result<Vec<TableInfo>, String> {
        ensure_config_ready(&self.app, connection).await?;
        dbx_core::schema::list_tables_core(&self.app, &connection.id, database, schema, None, None, None, None).await
    }

    async fn mongo_collection_stats(
        &self,
        connection: &ConnectionConfig,
        database: &str,
        collection: &str,
    ) -> Result<Value, String> {
        ensure_config_ready(&self.app, connection).await?;
        let stats = dbx_core::mongo_ops::mongo_collection_stats_core(
            &self.app,
            &connection.id,
            database,
            collection,
            None,
        )
        .await?;
        serde_json::to_value(stats).map_err(|error| error.to_string())
    }
}

async fn ensure_config_ready(app: &AppState, connection: &ConnectionConfig) -> Result<(), String> {
    {
        let mut configs = app.configs.write().await;
        configs.insert(connection.id.clone(), connection.clone());
    }
    app.reset_connection_transport_for_config(&connection.id, connection).await;
    app.get_or_create_pool(&connection.id, connection.database.as_deref()).await?;
    Ok(())
}

fn proxy_args_from_request(req: &DatabaseCatalogRequest) -> ProxyProfileRefArgs {
    ProxyProfileRefArgs {
        proxy_profile_id: req.proxy_profile_id.clone(),
        proxy_profile_name: req.proxy_profile_name.clone(),
        proxy_profile_ids: req.proxy_profile_ids.clone(),
        proxy_profile_names: req.proxy_profile_names.clone(),
    }
}

fn collect_connection_ids(req: &DatabaseCatalogRequest) -> Result<Vec<String>, AppError> {
    let mut ids = Vec::new();
    if let Some(id) = req.connection_id.as_deref().map(str::trim).filter(|v| !v.is_empty()) {
        ids.push(id.to_string());
    }
    if let Some(list) = &req.connection_ids {
        for id in list {
            let trimmed = id.trim();
            if !trimmed.is_empty() && !ids.iter().any(|existing| existing == trimmed) {
                ids.push(trimmed.to_string());
            }
        }
    }
    if ids.is_empty() {
        return Err(AppError::bad_request("connectionId or connectionIds is required"));
    }
    Ok(ids)
}

async fn load_connection(app: &AppState, connection_id: &str) -> Result<ConnectionConfig, AppError> {
    if let Some(config) = app.configs.read().await.get(connection_id).cloned() {
        return Ok(config);
    }
    let configs = app.storage.load_connections().await.map_err(AppError)?;
    configs
        .into_iter()
        .find(|config| config.id == connection_id)
        .ok_or_else(|| AppError::bad_request(format!("Connection \"{connection_id}\" not found")))
}

async fn apply_proxy_override(
    app: &AppState,
    config: ConnectionConfig,
    args: &ProxyProfileRefArgs,
) -> Result<ConnectionConfig, AppError> {
    if !has_proxy_profile_ref(args) {
        return Ok(config);
    }
    let has_id = args.proxy_profile_id.as_deref().is_some_and(|v| !v.trim().is_empty())
        || args.proxy_profile_ids.as_ref().is_some_and(|v| v.iter().any(|s| !s.trim().is_empty()));
    let has_name = args.proxy_profile_name.as_deref().is_some_and(|v| !v.trim().is_empty())
        || args.proxy_profile_names.as_ref().is_some_and(|v| v.iter().any(|s| !s.trim().is_empty()));
    if has_id && has_name {
        return Err(AppError::bad_request(
            "Specify either proxyProfileId(s) or proxyProfileName(s), not both.",
        ));
    }
    let profiles = app.storage.load_tunnel_profiles().await.map_err(AppError)?;
    let resolved = resolve_proxy_profiles(&profiles, args).map_err(|message| AppError::bad_request(message))?;
    if resolved.is_empty() {
        return Err(AppError::bad_request(
            "Proxy profile not found. Use GET /api/tunnel-profiles/list for saved profiles.",
        ));
    }
    let mut proxy_configs = Vec::with_capacity(resolved.len());
    for profile in &resolved {
        let TransportLayerConfig::Proxy(proxy) = profile else {
            return Err(AppError::bad_request("Selected tunnel profile is not a proxy profile."));
        };
        proxy_configs.push(proxy);
    }
    Ok(apply_proxy_profiles_failover(config, &proxy_configs))
}

fn batch_heading(config: &ConnectionConfig, index: usize, total: usize) -> String {
    if total <= 1 {
        String::new()
    } else {
        format!("## [{index}/{total}] {} ({})\n\n", config.name, config.id)
    }
}

fn finish_batch_parts(parts: Vec<String>, total: usize, skipped: usize, failures: usize) -> DatabaseCatalogResponse {
    let success = total.saturating_sub(skipped + failures);
    let mut markdown = parts.join("\n\n");
    if total > 1 {
        if !markdown.is_empty() {
            markdown.push_str("\n\n");
        }
        markdown.push_str(&format!(
            "---\nBatch: {success} success, {skipped} skipped, {failures} failures (total {total})"
        ));
    }
    DatabaseCatalogResponse { markdown, total, success, skipped, failures }
}

async fn run_stats_or_report(
    state: &WebState,
    req: DatabaseCatalogRequest,
    report: bool,
) -> Result<DatabaseCatalogResponse, AppError> {
    let ids = collect_connection_ids(&req)?;
    let proxy_args = proxy_args_from_request(&req);
    let executor = AppCatalogExecutor { app: Arc::clone(&state.app) };
    let options = DatabaseStatsOptions {
        database: req.database.clone(),
        schema: req.schema.clone(),
        redis_db: req.redis_db,
        timeout_ms: req.timeout_ms,
    };

    let mut parts = Vec::new();
    let mut skipped = 0usize;
    let mut failures = 0usize;
    let total = ids.len();

    for (idx, connection_id) in ids.into_iter().enumerate() {
        let loaded = load_connection(&state.app, &connection_id).await?;
        let config = apply_proxy_override(&state.app, loaded, &proxy_args).await?;
        let heading = batch_heading(&config, idx + 1, total);
        let result = if report {
            dbx_core::database_report::fetch_database_report(&executor, &config, options.clone()).await
        } else {
            dbx_core::database_stats::fetch_database_stats(&executor, &config, options.clone()).await
        };
        match result {
            Ok(body) => parts.push(format!("{heading}{body}")),
            Err(DatabaseStatsError { code: "UNSUPPORTED_DB_TYPE", message }) if req.skip_unsupported => {
                skipped += 1;
                parts.push(format!("{heading}Skipped [UNSUPPORTED_DB_TYPE]: {message}"));
            }
            Err(DatabaseStatsError { code, message }) => {
                failures += 1;
                parts.push(format!("{heading}Error [{code}]: {message}"));
            }
        }
    }

    Ok(finish_batch_parts(parts, total, skipped, failures))
}

pub async fn database_stats(
    State(state): State<Arc<WebState>>,
    Json(req): Json<DatabaseCatalogRequest>,
) -> Result<Json<DatabaseCatalogResponse>, AppError> {
    run_stats_or_report(&state, req, false).await.map(Json)
}

pub async fn database_report(
    State(state): State<Arc<WebState>>,
    Json(req): Json<DatabaseCatalogRequest>,
) -> Result<Json<DatabaseCatalogResponse>, AppError> {
    run_stats_or_report(&state, req, true).await.map(Json)
}
