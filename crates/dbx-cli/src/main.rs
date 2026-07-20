use std::{env, path::PathBuf, process::ExitCode, sync::Arc};

use dbx_core::{
    models::connection::{ConnectionConfig, DatabaseType},
    production_safety::{is_production_database, targets_production_database},
    sql_risk::{classify_sql_risk_for_database, SqlRisk},
    types::{ColumnInfo, QueryResult, TableInfo},
};
use dbx_mcp::{
    mongo::{self, MongoSafetyError},
    DbxBackend, LocalBackend, WebBackend,
};
use serde::Serialize;
use serde_json::{json, Map, Value};

const VERSION: &str = env!("CARGO_PKG_VERSION");
const DIRECT_QUERY_TYPES: &[&str] =
    &["postgres", "redshift", "mysql", "doris", "starrocks", "manticoresearch", "sqlite", "rqlite", "kwdb", "questdb"];
const BRIDGE_REQUIRED_TYPES: &[&str] = &[
    "cloudflare-d1",
    "redis",
    "mongodb",
    "duckdb",
    "clickhouse",
    "sqlserver",
    "oracle",
    "elasticsearch",
    "qdrant",
    "milvus",
    "weaviate",
    "chromadb",
    "etcd",
    "dameng",
    "kingbase",
    "highgo",
    "vastbase",
    "goldendb",
    "databend",
    "gaussdb",
    "yashandb",
    "databricks",
    "saphana",
    "teradata",
    "vertica",
    "firebird",
    "exasol",
    "opengauss",
    "oceanbase-oracle",
    "gbase",
    "tdengine",
    "iotdb",
    "h2",
    "snowflake",
    "trino",
    "prestosql",
    "hive",
    "spark",
    "db2",
    "informix",
    "iris",
    "neo4j",
    "cassandra",
    "bigquery",
    "kylin",
    "sundb",
    "oscar",
    "xugu",
    "jdbc",
    "access",
    "influxdb",
    "zookeeper",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum OutputFormat {
    Table,
    Json,
    Csv,
}

#[derive(Debug)]
struct Flags {
    args: Vec<String>,
    format: OutputFormat,
    schema: Option<String>,
    database: Option<String>,
    tables: Vec<String>,
    max_tables: Option<usize>,
    max_rows: Option<usize>,
    timeout_ms: Option<u64>,
    file: Option<PathBuf>,
    allow_writes: bool,
    allow_dangerous: bool,
    help: bool,
    version: bool,
    quiet: bool,
    verbose: bool,
    /// None = sequential; Some(n) = parallel (n==0 → default 15).
    parallel: Option<usize>,
    skip_unsupported: bool,
    no_save: bool,
    output: Option<PathBuf>,
    name: Option<String>,
    db_type: Option<String>,
    host: Option<String>,
    port: Option<u16>,
    username: Option<String>,
    password: Option<String>,
    ssl: bool,
    driver_profile: Option<String>,
    proxy: bool,
    proxy_type: Option<String>,
    proxy_host: Option<String>,
    proxy_port: Option<u16>,
    proxy_username: Option<String>,
    proxy_password: Option<String>,
    proxy_profile_id: Option<String>,
    proxy_profile_name: Option<String>,
}

#[derive(Debug)]
enum CliOutcome {
    Ok(String),
    SoftFail(String),
}

#[derive(Debug)]
struct CliError {
    code: &'static str,
    message: String,
}

impl CliError {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self { code, message: message.into() }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Diagnostics {
    app_data_dir: String,
    db_path: String,
    db_path_exists: bool,
    connections_table_exists: bool,
    connection_row_count: usize,
    load_connections_ok: bool,
    loaded_connection_count: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    load_connections_error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    load_connections_hint: Option<String>,
    bridge_port_file: String,
    bridge_port_file_exists: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    bridge_url: Option<String>,
    direct_query_types: Vec<&'static str>,
    bridge_required_types: Vec<&'static str>,
}

#[tokio::main]
async fn main() -> ExitCode {
    match run(env::args().skip(1).collect()).await {
        Ok(CliOutcome::Ok(output)) => {
            print!("{output}");
            ExitCode::SUCCESS
        }
        Ok(CliOutcome::SoftFail(output)) => {
            print!("{output}");
            ExitCode::FAILURE
        }
        Err((error, json_output)) => {
            if json_output {
                eprintln!(
                    "{}",
                    serde_json::to_string_pretty(&json!({ "error": { "code": error.code, "message": error.message } }))
                        .unwrap()
                );
            } else {
                eprintln!("Error [{}]: {}", error.code, error.message);
            }
            ExitCode::FAILURE
        }
    }
}

async fn run(argv: Vec<String>) -> Result<CliOutcome, (CliError, bool)> {
    let wants_json = argv.iter().any(|arg| arg == "--json" || arg == "-j");
    let flags = parse_flags(&argv).map_err(|error| (error, wants_json))?;
    let json_output = flags.format == OutputFormat::Json;
    if flags.version {
        return Ok(CliOutcome::Ok(format!("{VERSION}\n")));
    }
    if flags.args.is_empty() || flags.help || flags.args.first().is_some_and(|arg| arg == "help") {
        return Ok(CliOutcome::Ok(format!("{}\n", usage())));
    }
    if flags.args[0] == "doctor" {
        ensure_arg_count(&flags.args, 1, "dbx doctor").map_err(|error| (error, json_output))?;
        let diagnostics = diagnostics().await;
        return format_diagnostics(&diagnostics, flags.format)
            .map(CliOutcome::Ok)
            .map_err(|error| (error, json_output));
    }
    if flags.args[0] == "capabilities" {
        ensure_arg_count(&flags.args, 1, "dbx capabilities").map_err(|error| (error, json_output))?;
        return format_capabilities(flags.format)
            .map(CliOutcome::Ok)
            .map_err(|error| (error, json_output));
    }

    let backend: Arc<dyn DbxBackend> = if let Ok(base_url) = env::var("DBX_WEB_URL") {
        Arc::new(
            WebBackend::new(base_url, env::var("DBX_WEB_PASSWORD").unwrap_or_default())
                .map_err(|message| (CliError::new("CONNECTION_STORE_ERROR", message), json_output))?,
        )
    } else {
        let db_path = dbx_mcp::paths::storage_db_path()
            .map_err(|message| (CliError::new("CONNECTION_STORE_ERROR", message), json_output))?;
        Arc::new(
            LocalBackend::open(&db_path)
                .await
                .map_err(|message| (CliError::new("CONNECTION_STORE_ERROR", message), json_output))?,
        )
    };

    run_with_backend(backend, flags).await.map_err(|error| (error, json_output))
}

fn ok(body: String) -> Result<CliOutcome, CliError> {
    Ok(CliOutcome::Ok(body))
}

fn soft_fail(body: String) -> Result<CliOutcome, CliError> {
    Ok(CliOutcome::SoftFail(body))
}

fn outcome_text(outcome: CliOutcome) -> String {
    match outcome {
        CliOutcome::Ok(body) | CliOutcome::SoftFail(body) => body,
    }
}

async fn run_with_backend(backend: Arc<dyn DbxBackend>, flags: Flags) -> Result<CliOutcome, CliError> {
    let args = &flags.args;
    if args.first().is_some_and(|arg| arg == "connections") && args.get(1).is_some_and(|arg| arg == "list") {
        ensure_arg_count(args, 2, "dbx connections list")?;
        return format_connections(&backend.load_connections().await.map_err(store_error)?, flags.format).map(CliOutcome::Ok);
    }
    if args.first().is_some_and(|arg| arg == "connections") && args.get(1).is_some_and(|arg| arg == "add") {
        return run_connections_add(backend.as_ref(), &flags).await;
    }
    if args.first().is_some_and(|arg| arg == "connections") && args.get(1).is_some_and(|arg| arg == "remove") {
        ensure_arg_count(args, 3, "dbx connections remove")?;
        let connection_ref = required(args.get(2), "Connection name or list index is required.")?;
        let connection = find_connection(backend.as_ref(), connection_ref).await?;
        let removed = backend
            .remove_connection_for_mcp(&connection.id)
            .await
            .map_err(store_error)?;
        if !removed {
            return Err(CliError::new(
                "CONNECTION_NOT_FOUND",
                format!("Connection \"{}\" was not removed.", connection.name),
            ));
        }
        if flags.format == OutputFormat::Json {
            return ok(format!(
                "{}\n",
                serde_json::to_string_pretty(&json!({ "removed": true, "id": connection.id, "name": connection.name }))
                    .unwrap()
            ));
        }
        return ok(format!("Connection \"{}\" removed.\n", connection.name));
    }
    if args.first().is_some_and(|arg| arg == "schema") && args.get(1).is_some_and(|arg| arg == "list") {
        ensure_arg_count(args, 3, "dbx schema list")?;
        return run_schema_list(backend, &flags).await;
    }
    if args.first().is_some_and(|arg| arg == "schema") && args.get(1).is_some_and(|arg| arg == "describe") {
        ensure_arg_count(args, 4, "dbx schema describe")?;
        return run_schema_describe(backend, &flags).await;
    }
    if args.first().is_some_and(|arg| arg == "query") {
        return run_query(backend, &flags).await;
    }
    if args.first().is_some_and(|arg| arg == "redis") {
        return run_redis(backend, &flags).await;
    }
    if args.first().is_some_and(|arg| arg == "context") {
        return run_context(backend, &flags).await;
    }
    if args.first().is_some_and(|arg| arg == "proxies") && args.get(1).is_some_and(|arg| arg == "list") {
        ensure_arg_count(args, 2, "dbx proxies list")?;
        let profiles = backend.load_tunnel_profiles().await.map_err(store_error)?;
        return ok(format!("{}\n", dbx_mcp::tunnel_profiles::format_proxy_list(&profiles)));
    }
    if args.first().is_some_and(|arg| arg == "stats") {
        ensure_arg_count(args, 2, "dbx stats")?;
        return run_stats_or_report(backend, &flags, false).await;
    }
    if args.first().is_some_and(|arg| arg == "report") {
        ensure_arg_count(args, 2, "dbx report")?;
        return run_stats_or_report(backend, &flags, true).await;
    }
    if args.first().is_some_and(|arg| arg == "open") {
        ensure_arg_count(args, 3, "dbx open")?;
        return run_open(backend, &flags).await;
    }
    Err(CliError::new("USAGE", usage()))
}

async fn select_connections(backend: &dyn DbxBackend, connection_ref: &str) -> Result<Vec<ConnectionConfig>, CliError> {
    let connections = backend.load_connections().await.map_err(store_error)?;
    if let Ok(Some(indexes)) = dbx_mcp::list_index::parse_list_index_range(connection_ref) {
        if indexes.len() > dbx_mcp::list_index::MAX_LIST_INDEX_RANGE_WARN_SIZE {
            eprintln!(
                "[dbx] Warning: range \"{connection_ref}\" resolves to {} connections (>{}).",
                indexes.len(),
                dbx_mcp::list_index::MAX_LIST_INDEX_RANGE_WARN_SIZE
            );
        }
        let mut out = Vec::new();
        for index in indexes {
            let connection = connections.get(index - 1).cloned().ok_or_else(|| {
                CliError::new(
                    "CONNECTION_NOT_FOUND",
                    format!("List index #{index} is out of range (1-{}).", connections.len()),
                )
            })?;
            out.push(connection);
        }
        return Ok(out);
    }
    Ok(vec![find_connection(backend, connection_ref).await?])
}

async fn apply_cli_proxy_override(
    backend: &dyn DbxBackend,
    configs: Vec<ConnectionConfig>,
    flags: &Flags,
) -> Result<Vec<ConnectionConfig>, CliError> {
    let has_ref = flags.proxy_profile_id.as_deref().is_some_and(|v| !v.trim().is_empty())
        || flags.proxy_profile_name.as_deref().is_some_and(|v| !v.trim().is_empty());
    if !has_ref {
        return Ok(configs);
    }
    let mut out = Vec::with_capacity(configs.len());
    for config in configs {
        let config = dbx_mcp::resolve::apply_proxy_override_if_requested(
            backend,
            config,
            flags.proxy_profile_id.clone(),
            flags.proxy_profile_name.clone(),
        )
        .await
        .map_err(|error| {
            CliError::new(
                "PROXY_OVERRIDE_ERROR",
                error
                    .content
                    .first()
                    .and_then(|block| block.as_text())
                    .map(|text| text.text.clone())
                    .unwrap_or_else(|| "Proxy override failed".into()),
            )
        })?;
        out.push(config);
    }
    Ok(out)
}

fn dynamic_cli_error(code: &str, message: impl Into<String>) -> CliError {
    let static_code: &'static str = match code {
        "SQL_BLOCKED" => "SQL_BLOCKED",
        "QUERY_ERROR" => "QUERY_ERROR",
        "REDIS_COMMAND_REQUIRED" => "REDIS_COMMAND_REQUIRED",
        "REDIS_COMMAND_ERROR" => "REDIS_COMMAND_ERROR",
        "INVALID_CONNECTION_TYPE" => "INVALID_CONNECTION_TYPE",
        "INVALID_OPTION" => "INVALID_OPTION",
        "INVALID_ARGUMENT" => "INVALID_ARGUMENT",
        "CONNECTION_NOT_FOUND" => "CONNECTION_NOT_FOUND",
        "CONNECTION_STORE_ERROR" => "CONNECTION_STORE_ERROR",
        "DBX_NOT_RUNNING" => "DBX_NOT_RUNNING",
        "UNSUPPORTED_DB_TYPE" => "UNSUPPORTED_DB_TYPE",
        "PROXY_OVERRIDE_ERROR" => "PROXY_OVERRIDE_ERROR",
        "ERROR" => "ERROR",
        _ => "ERROR",
    };
    CliError::new(static_code, message)
}

fn finish_batch_outcome<T>(
    items: &[dbx_mcp::batch::BatchItem<T>],
    selected: &[ConnectionConfig],
    mut parts: Vec<String>,
) -> Result<CliOutcome, CliError> {
    let total = selected.len();
    let (ok_count, skipped, failures) = dbx_mcp::batch::count_batch(items);
    if total == 1 && failures > 0 {
        if let Some(dbx_mcp::batch::BatchItem::Err { code, message, .. }) = items.first() {
            return Err(dynamic_cli_error(code, message.clone()));
        }
    }
    if total > 1 {
        parts.push(dbx_mcp::batch::batch_summary(total, ok_count, skipped, failures));
    }
    let joined = parts.join("\n\n");
    let body = if joined.ends_with('\n') {
        joined
    } else {
        format!("{joined}\n")
    };
    if failures > 0 {
        soft_fail(body)
    } else {
        ok(body)
    }
}

async fn run_batch_string_jobs<F, Fut>(
    backend: Arc<dyn DbxBackend>,
    selected: Vec<ConnectionConfig>,
    flags: &Flags,
    worker: F,
) -> Result<CliOutcome, CliError>
where
    F: Fn(Arc<dyn DbxBackend>, ConnectionConfig, usize) -> Fut + Send + Sync + 'static,
    Fut: std::future::Future<Output = Result<String, (String, String, bool)>> + Send + 'static,
{
    let quiet = flags.quiet;
    let verbose = flags.verbose;
    let items = dbx_mcp::batch::run_connection_batch(&selected, flags.parallel, move |config, index| {
        let backend = Arc::clone(&backend);
        let fut = worker(backend, config, index);
        async move {
            let _guard = dbx_mcp::progress::push_progress(dbx_mcp::progress::cli_progress_options(quiet, verbose));
            fut.await
        }
    })
    .await;

    let total = selected.len();
    let mut parts = Vec::new();
    for item in &items {
        match item {
            dbx_mcp::batch::BatchItem::Ok { index, value } => {
                let heading = dbx_mcp::batch::batch_heading(&selected[*index], index + 1, total);
                parts.push(format!("{heading}{value}"));
            }
            dbx_mcp::batch::BatchItem::Skipped { index, name, code, message } => {
                let heading = if total > 1 {
                    format!("## #{} {name}\n\n", index + 1)
                } else {
                    String::new()
                };
                parts.push(format!("{heading}Skipped [{code}]: {message}"));
            }
            dbx_mcp::batch::BatchItem::Err { index, name, code, message } => {
                let heading = if total > 1 {
                    format!("## #{} {name}\n\n", index + 1)
                } else {
                    String::new()
                };
                parts.push(format!("{heading}Error [{code}]: {message}"));
            }
        }
    }
    finish_batch_outcome(&items, &selected, parts)
}

async fn run_stats_or_report(
    backend: Arc<dyn DbxBackend>,
    flags: &Flags,
    report: bool,
) -> Result<CliOutcome, CliError> {
    let connection_ref = required(flags.args.get(1), "Connection name or list index/range is required.")?;
    let selected = select_connections(backend.as_ref(), connection_ref).await?;
    let selected = apply_cli_proxy_override(backend.as_ref(), selected, flags).await?;
    let total = selected.len();
    let database = flags.database.clone();
    let schema = flags.schema.clone();
    let timeout_ms = flags.timeout_ms;
    let skip_unsupported = flags.skip_unsupported;
    let quiet = flags.quiet;
    let verbose = flags.verbose;

    let items = dbx_mcp::batch::run_connection_batch(&selected, flags.parallel, move |config, _index| {
        let backend = Arc::clone(&backend);
        let database = database.clone();
        let schema = schema.clone();
        async move {
            let _guard = dbx_mcp::progress::push_progress(dbx_mcp::progress::cli_progress_options(quiet, verbose));
            dbx_mcp::progress::log_using_connection(&config);
            let options = dbx_mcp::database_stats::DatabaseStatsOptions {
                database,
                schema,
                redis_db: None,
                timeout_ms,
            };
            let result = if report {
                dbx_mcp::database_report::fetch_database_report(backend.as_ref(), &config, options).await
            } else {
                dbx_mcp::database_stats::fetch_database_stats(backend.as_ref(), &config, options).await
            };
            match result {
                Ok(body) => Ok(body),
                Err(error) if skip_unsupported && error.code == "UNSUPPORTED_DB_TYPE" => {
                    Err((error.code.to_string(), error.message, true))
                }
                Err(error) => Err((error.code.to_string(), error.message, false)),
            }
        }
    })
    .await;

    let mut parts = Vec::new();
    let mut saved_paths = Vec::new();
    for item in &items {
        match item {
            dbx_mcp::batch::BatchItem::Ok { index, value } => {
                let config = &selected[*index];
                let heading = dbx_mcp::batch::batch_heading(config, index + 1, total);
                if report && !flags.no_save {
                    let dir = flags
                        .output
                        .clone()
                        .unwrap_or_else(dbx_mcp::database_report::default_reports_dir);
                    std::fs::create_dir_all(&dir).map_err(|e| CliError::new("REPORT_SAVE_ERROR", e.to_string()))?;
                    let stamp = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_secs())
                        .unwrap_or(0);
                    let scope = flags
                        .database
                        .clone()
                        .or_else(|| flags.schema.clone())
                        .or_else(|| config.database.clone())
                        .unwrap_or_else(|| "default".to_string());
                    let path = if total > 1 {
                        let batch_dir = if flags.output.is_some() {
                            dir
                        } else {
                            let batch_dir = dir.join(format!("dbx-report-batch-{stamp}"));
                            std::fs::create_dir_all(&batch_dir)
                                .map_err(|e| CliError::new("REPORT_SAVE_ERROR", e.to_string()))?;
                            batch_dir
                        };
                        batch_dir.join(format!("dbx-report-{}-{}.md", config.name, scope))
                    } else if let Some(output) = &flags.output {
                        output.clone()
                    } else {
                        dir.join(format!("dbx-report-{}-{}-{}.md", config.name, scope, stamp))
                    };
                    if let Some(parent) = path.parent() {
                        std::fs::create_dir_all(parent)
                            .map_err(|e| CliError::new("REPORT_SAVE_ERROR", e.to_string()))?;
                    }
                    std::fs::write(&path, value).map_err(|e| CliError::new("REPORT_SAVE_ERROR", e.to_string()))?;
                    saved_paths.push(path.display().to_string());
                }
                parts.push(format!("{heading}{value}"));
            }
            dbx_mcp::batch::BatchItem::Skipped { index, name, code, message } => {
                let heading = if total > 1 {
                    format!("## #{} {name}\n\n", index + 1)
                } else {
                    String::new()
                };
                parts.push(format!("{heading}Skipped [{code}]: {message}"));
            }
            dbx_mcp::batch::BatchItem::Err { index, name, code, message } => {
                let heading = if total > 1 {
                    format!("## #{} {name}\n\n", index + 1)
                } else {
                    String::new()
                };
                parts.push(format!("{heading}Error [{code}]: {message}"));
            }
        }
    }
    for path in &saved_paths {
        eprintln!("[dbx] Report saved: {path}");
    }
    finish_batch_outcome(&items, &selected, parts)
}

async fn run_query(backend: Arc<dyn DbxBackend>, flags: &Flags) -> Result<CliOutcome, CliError> {
    let args = &flags.args;
    let default_connection = env::var("DBX_CONNECTION").ok().filter(|value| !value.is_empty());
    let uses_default = default_connection.is_some() && args.len() == if flags.file.is_some() { 1 } else { 2 };
    ensure_arg_count(
        args,
        if uses_default {
            if flags.file.is_some() {
                1
            } else {
                2
            }
        } else if flags.file.is_some() {
            2
        } else {
            3
        },
        "dbx query",
    )?;
    let connection_ref = if uses_default {
        default_connection.as_deref().unwrap()
    } else {
        required(args.get(1), "Connection name is required.")?
    };
    if flags.file.is_some() && args.get(2).is_some() {
        return Err(CliError::new("INVALID_ARGUMENT", "Provide SQL either inline or with --file, not both."));
    }
    let sql = if let Some(file) = &flags.file {
        tokio::fs::read_to_string(file).await.map_err(|error| CliError::new("ERROR", error.to_string()))?
    } else {
        required(args.get(if uses_default { 1 } else { 2 }), "SQL string or --file is required.")?.to_string()
    };
    let env_allow_writes = env_flag("DBX_MCP_ALLOW_WRITES");
    let env_allow_dangerous = env_flag("DBX_MCP_ALLOW_DANGEROUS_SQL");
    if flags.allow_dangerous && !flags.allow_writes && !env_allow_writes {
        return Err(CliError::new("INVALID_OPTION", "--allow-dangerous-sql requires --allow-writes."));
    }
    let allow_writes = flags.allow_writes || env_allow_writes;
    let allow_dangerous = flags.allow_dangerous || env_allow_dangerous;
    let selected = select_connections(backend.as_ref(), connection_ref).await?;
    let selected = apply_cli_proxy_override(backend.as_ref(), selected, flags).await?;
    let format = flags.format;
    let database_override = flags.database.clone();
    let max_rows = flags.max_rows;
    let timeout_ms = flags.timeout_ms;
    let sql_owned = sql.clone();

    run_batch_string_jobs(backend, selected, flags, move |backend, connection, _index| {
        let sql = sql_owned.clone();
        let database_override = database_override.clone();
        async move {
            dbx_mcp::progress::log_using_connection(&connection);
            dbx_mcp::progress::log_query_sql(&sql);
            match execute_query_one(
                backend.as_ref(),
                &connection,
                &sql,
                database_override.as_deref(),
                allow_writes,
                allow_dangerous,
                max_rows,
                timeout_ms,
                format,
            )
            .await
            {
                Ok(body) => Ok(body),
                Err(error) => Err((error.code.to_string(), error.message, false)),
            }
        }
    })
    .await
}

async fn execute_query_one(
    backend: &dyn DbxBackend,
    connection: &ConnectionConfig,
    sql: &str,
    database_override: Option<&str>,
    allow_writes: bool,
    allow_dangerous: bool,
    max_rows: Option<usize>,
    timeout_ms: Option<u64>,
    format: OutputFormat,
) -> Result<String, CliError> {
    let database = selected_database(connection, database_override);
    if connection.db_type == DatabaseType::Redis {
        return Err(CliError::new(
            "REDIS_COMMAND_REQUIRED",
            "Redis connections do not accept SQL through dbx query. Use dbx redis.",
        ));
    }
    if connection.db_type == DatabaseType::MongoDb {
        let command = mongo::parse(sql).map_err(|message| CliError::new("QUERY_ERROR", message))?;
        if let Err(error) = mongo::validate_safety(
            &command,
            allow_writes,
            allow_dangerous,
            is_production_database(connection, &database),
        ) {
            return Err(match error {
                MongoSafetyError::WritesDisabled => {
                    CliError::new("SQL_BLOCKED", "MongoDB write command is blocked. Pass --allow-writes to allow it.")
                }
                MongoSafetyError::EmptyFilter => CliError::new(
                    "SQL_BLOCKED",
                    "MongoDB update/delete commands require a non-empty filter unless --allow-dangerous-sql is set.",
                ),
                MongoSafetyError::Dangerous => CliError::new(
                    "SQL_BLOCKED",
                    "Dangerous MongoDB command is blocked. Pass --allow-dangerous-sql to allow it.",
                ),
                MongoSafetyError::ProductionWrite => {
                    CliError::new("SQL_BLOCKED", "Writes and DDL are blocked for production databases.")
                }
            });
        }
        let mut result =
            backend.execute_mongo_command(connection, &database, &command).await.map_err(command_error)?;
        truncate_query_result(&mut result, max_rows);
        return format_query(&connection.name, &result, format);
    }
    let risk = classify_sql_risk_for_database(sql, connection.db_type)
        .map_err(|message| CliError::new("SQL_BLOCKED", message))?;
    if risk == SqlRisk::Transaction
        || risk == SqlRisk::Write && !allow_writes
        || risk == SqlRisk::Ddl && !allow_dangerous
    {
        return Err(CliError::new("SQL_BLOCKED", format!("{risk} statement is blocked.")));
    }
    if risk != SqlRisk::ReadOnly && targets_production_database(connection, &database, sql) {
        return Err(CliError::new("SQL_BLOCKED", "Writes and DDL are blocked for production databases."));
    }
    let timeout_secs = timeout_ms.map(|value| value.div_ceil(1000));
    let result = backend
        .execute_query(connection, &database, sql, max_rows, timeout_secs)
        .await
        .map_err(command_error)?;
    format_query(&connection.name, &result, format)
}

fn truncate_query_result(result: &mut QueryResult, max_rows: Option<usize>) {
    let Some(max_rows) = max_rows else { return };
    if result.rows.len() > max_rows {
        result.rows.truncate(max_rows);
        result.truncated = true;
    }
}

async fn run_schema_list(backend: Arc<dyn DbxBackend>, flags: &Flags) -> Result<CliOutcome, CliError> {
    let connection_ref = required(flags.args.get(2), "Connection name is required.")?;
    let selected = select_connections(backend.as_ref(), connection_ref).await?;
    let selected = apply_cli_proxy_override(backend.as_ref(), selected, flags).await?;
    let format = flags.format;
    let database_override = flags.database.clone();
    let schema = flags.schema.clone();
    run_batch_string_jobs(backend, selected, flags, move |backend, connection, _index| {
        let database_override = database_override.clone();
        let schema = schema.clone();
        async move {
            dbx_mcp::progress::log_using_connection(&connection);
            let database = selected_database(&connection, database_override.as_deref());
            let schema = schema.as_deref().unwrap_or("");
            match backend.list_tables(&connection, &database, schema).await {
                Ok(tables) => match format_tables(&connection.name, flags_schema(schema), &tables, format) {
                    Ok(body) => Ok(body),
                    Err(error) => Err((error.code.to_string(), error.message, false)),
                },
                Err(error) => Err(("ERROR".into(), error, false)),
            }
        }
    })
    .await
}

fn flags_schema(schema: &str) -> Option<&str> {
    if schema.is_empty() {
        None
    } else {
        Some(schema)
    }
}

async fn run_schema_describe(backend: Arc<dyn DbxBackend>, flags: &Flags) -> Result<CliOutcome, CliError> {
    let connection_ref = required(flags.args.get(2), "Connection name is required.")?;
    let table = required(flags.args.get(3), "Table name is required.")?.to_string();
    let selected = select_connections(backend.as_ref(), connection_ref).await?;
    let selected = apply_cli_proxy_override(backend.as_ref(), selected, flags).await?;
    let format = flags.format;
    let database_override = flags.database.clone();
    let schema = flags.schema.clone();
    run_batch_string_jobs(backend, selected, flags, move |backend, connection, _index| {
        let database_override = database_override.clone();
        let schema = schema.clone();
        let table = table.clone();
        async move {
            dbx_mcp::progress::log_using_connection(&connection);
            let database = selected_database(&connection, database_override.as_deref());
            let schema_name = schema.as_deref().unwrap_or("");
            match backend.get_columns(&connection, &database, schema_name, &table).await {
                Ok(columns) => {
                    match format_columns(&connection.name, flags_schema(schema_name), &table, &columns, format) {
                        Ok(body) => Ok(body),
                        Err(error) => Err((error.code.to_string(), error.message, false)),
                    }
                }
                Err(error) => Err(("ERROR".into(), error, false)),
            }
        }
    })
    .await
}

async fn run_context(backend: Arc<dyn DbxBackend>, flags: &Flags) -> Result<CliOutcome, CliError> {
    let args = &flags.args;
    let default_connection = env::var("DBX_CONNECTION").ok().filter(|value| !value.is_empty());
    let uses_default = default_connection.is_some() && args.len() == 1;
    ensure_arg_count(args, if uses_default { 1 } else { 2 }, "dbx context")?;
    if flags.format == OutputFormat::Csv {
        return Err(CliError::new("INVALID_OPTION", "CSV format is not supported for dbx context."));
    }
    let connection_ref = if uses_default {
        default_connection.as_deref().unwrap()
    } else {
        required(args.get(1), "Connection name is required.")?
    };
    let selected = select_connections(backend.as_ref(), connection_ref).await?;
    let selected = apply_cli_proxy_override(backend.as_ref(), selected, flags).await?;
    let format = flags.format;
    let database_override = flags.database.clone();
    let schema = flags.schema.clone();
    let tables = flags.tables.clone();
    let max_tables = flags.max_tables;
    run_batch_string_jobs(backend, selected, flags, move |backend, connection, _index| {
        let database_override = database_override.clone();
        let schema = schema.clone();
        let tables = tables.clone();
        async move {
            dbx_mcp::progress::log_using_connection(&connection);
            match build_context_body(
                backend.as_ref(),
                &connection,
                database_override.as_deref(),
                schema.as_deref(),
                &tables,
                max_tables,
                format,
            )
            .await
            {
                Ok(body) => Ok(body),
                Err(error) => Err((error.code.to_string(), error.message, false)),
            }
        }
    })
    .await
}

async fn build_context_body(
    backend: &dyn DbxBackend,
    connection: &ConnectionConfig,
    database_override: Option<&str>,
    schema: Option<&str>,
    requested_tables: &[String],
    max_tables: Option<usize>,
    format: OutputFormat,
) -> Result<String, CliError> {
    let database = selected_database(connection, database_override);
    let schema = schema.unwrap_or("");
    let all_tables = backend.list_tables(connection, &database, schema).await.map_err(command_error)?;
    let max_tables = max_tables.unwrap_or(8).clamp(1, 20);
    let requested = !requested_tables.is_empty();
    let selected: Vec<TableInfo> = if !requested {
        all_tables.iter().take(max_tables).cloned().collect()
    } else {
        all_tables
            .iter()
            .filter(|table| requested_tables.iter().any(|name| name.eq_ignore_ascii_case(&table.name)))
            .cloned()
            .collect()
    };
    let truncated = selected.len() > max_tables || (!requested && all_tables.len() > max_tables);
    let selected = selected.into_iter().take(max_tables).collect::<Vec<_>>();
    let mut context_tables = Vec::new();
    for table in selected {
        let columns = backend
            .get_columns(connection, &database, schema, &table.name)
            .await
            .map_err(command_error)?;
        context_tables.push(json!({ "name": table.name, "type": table.table_type, "columns": columns }));
    }
    let payload = json!({
        "connection": connection.name,
        "database": database,
        "schema": schema,
        "truncated": truncated,
        "tables": context_tables,
    });
    if format == OutputFormat::Json {
        return json_string(&payload);
    }
    let mut header = vec![format!("Connection: {}", connection.name)];
    if !database.is_empty() {
        header.push(format!("Database: {database}"));
    }
    if !schema.is_empty() {
        header.push(format!("Schema: {schema}"));
    }
    let mut output = format!("{}\n", header.join("\n"));
    for table in payload["tables"].as_array().unwrap() {
        output.push_str(&format!(
            "\n## {}\nType: {}\n",
            table["name"].as_str().unwrap_or_default(),
            table["type"].as_str().unwrap_or_default()
        ));
        for column in table["columns"].as_array().unwrap_or(&Vec::new()) {
            output.push_str(&format!(
                "- {} {} {}{}{}\n",
                column["name"].as_str().unwrap_or_default(),
                column["data_type"].as_str().unwrap_or_default(),
                if column["is_nullable"].as_bool().unwrap_or(false) {
                    "NULL"
                } else {
                    "NOT NULL"
                },
                if column["is_primary_key"].as_bool().unwrap_or(false) {
                    " PK"
                } else {
                    ""
                },
                column["comment"]
                    .as_str()
                    .map(|comment| format!(" -- {comment}"))
                    .unwrap_or_default()
            ));
        }
    }
    if truncated {
        output.push_str("\nNote: table list was truncated; request specific table names for more context.\n");
    }
    Ok(output)
}

async fn run_open(backend: Arc<dyn DbxBackend>, flags: &Flags) -> Result<CliOutcome, CliError> {
    if flags.format == OutputFormat::Csv {
        return Err(CliError::new("INVALID_OPTION", "CSV format is not supported for dbx open."));
    }
    let connection_ref = required(flags.args.get(1), "Connection name is required.")?;
    let table = required(flags.args.get(2), "Table name is required.")?.to_string();
    let selected = select_connections(backend.as_ref(), connection_ref).await?;
    let total = selected.len();
    let mut parts = Vec::new();
    let mut failures = 0usize;
    let quiet = flags.quiet;
    let verbose = flags.verbose;
    for (idx, connection) in selected.into_iter().enumerate() {
        let _guard = dbx_mcp::progress::push_progress(dbx_mcp::progress::cli_progress_options(quiet, verbose));
        dbx_mcp::progress::log_using_connection(&connection);
        let heading = dbx_mcp::batch::batch_heading(&connection, idx + 1, total);
        match backend
            .bridge_request(
                "/open-table",
                optional_object([
                    ("connection_name", Some(json!(connection.name))),
                    ("table", Some(json!(table))),
                    ("schema", flags.schema.clone().map(|value| json!(value))),
                    ("database", flags.database.clone().map(|value| json!(value))),
                ]),
            )
            .await
        {
            Ok(()) => {
                if flags.format == OutputFormat::Json {
                    parts.push(format!(
                        "{heading}{}",
                        serde_json::to_string_pretty(&optional_object([
                            ("opened", Some(json!(true))),
                            ("connection", Some(json!(connection.name))),
                            ("table", Some(json!(table))),
                            ("schema", flags.schema.clone().map(|value| json!(value))),
                            ("database", flags.database.clone().map(|value| json!(value))),
                        ]))
                        .unwrap()
                    ));
                } else {
                    parts.push(format!("{heading}Opened {table} in DBX"));
                }
            }
            Err(message) => {
                failures += 1;
                if total == 1 {
                    return Err(CliError::new("DBX_NOT_RUNNING", message));
                }
                parts.push(format!("{heading}Error [DBX_NOT_RUNNING]: {message}"));
            }
        }
    }
    let _ = failures;
    let mut ok_count = 0usize;
    let mut fail_count = 0usize;
    // Reconstruct counts from parts is awkward; recompute from loop state.
    // `failures` already tracked.
    fail_count = failures;
    ok_count = total.saturating_sub(failures);
    if total > 1 {
        parts.push(dbx_mcp::batch::batch_summary(total, ok_count, 0, fail_count));
    }
    let joined = parts.join("\n\n");
    let body = if joined.ends_with('\n') {
        joined
    } else {
        format!("{joined}\n")
    };
    if fail_count > 0 {
        soft_fail(body)
    } else {
        ok(body)
    }
}

async fn run_redis(backend: Arc<dyn DbxBackend>, flags: &Flags) -> Result<CliOutcome, CliError> {
    if flags.args.len() < 3 {
        return Err(CliError::new(
            "INVALID_ARGUMENT",
            "dbx redis expects <connection|#|range> <command...>",
        ));
    }
    if flags.format == OutputFormat::Csv {
        return Err(CliError::new("INVALID_OPTION", "CSV format is not supported for dbx redis."));
    }
    let connection_ref = &flags.args[1];
    let command = flags.args[2..].join(" ");
    let selected = select_connections(backend.as_ref(), connection_ref).await?;
    let selected = apply_cli_proxy_override(backend.as_ref(), selected, flags).await?;
    let allow_dangerous = flags.allow_dangerous;
    let database_override = flags.database.clone();
    let format = flags.format;
    run_batch_string_jobs(backend, selected, flags, move |backend, connection, _index| {
        let command = command.clone();
        let database_override = database_override.clone();
        async move {
            dbx_mcp::progress::log_using_connection(&connection);
            if connection.db_type != DatabaseType::Redis {
                return Err((
                    "INVALID_CONNECTION_TYPE".into(),
                    format!("Connection \"{}\" is not Redis.", connection.name),
                    false,
                ));
            }
            let db = database_override
                .as_deref()
                .and_then(|v| v.parse().ok())
                .or_else(|| connection.database.as_deref().and_then(|v| v.parse().ok()))
                .unwrap_or(0u32);
            match backend
                .execute_redis_command(&connection, db, &command, allow_dangerous)
                .await
            {
                Ok(result) => {
                    if format == OutputFormat::Json {
                        Ok(serde_json::to_string_pretty(&json!({
                            "connection": connection.name,
                            "db": db,
                            "value": result.value,
                        }))
                        .unwrap())
                    } else if let Value::String(text) = &result.value {
                        Ok(text.clone())
                    } else {
                        Ok(result.value.to_string())
                    }
                }
                Err(error) => Err(("REDIS_COMMAND_ERROR".into(), error, false)),
            }
        }
    })
    .await
}

async fn run_connections_add(backend: &dyn DbxBackend, flags: &Flags) -> Result<CliOutcome, CliError> {
    ensure_arg_count(&flags.args, 2, "dbx connections add")?;
    let name = flags
        .name
        .as_deref()
        .filter(|v| !v.is_empty())
        .ok_or_else(|| CliError::new("INVALID_ARGUMENT", "--name is required."))?;
    let db_type = flags
        .db_type
        .as_deref()
        .filter(|v| !v.is_empty())
        .ok_or_else(|| CliError::new("INVALID_ARGUMENT", "--type is required."))?;
    let host = flags
        .host
        .as_deref()
        .filter(|v| !v.is_empty())
        .ok_or_else(|| CliError::new("INVALID_ARGUMENT", "--host is required."))?;
    let existing = backend.load_connections().await.map_err(store_error)?;
    if existing.iter().any(|c| c.name.eq_ignore_ascii_case(name)) {
        return Err(CliError::new("CONNECTION_EXISTS", format!("Connection \"{name}\" already exists.")));
    }
    let parsed =
        dbx_mcp::backend::parse_database_type(db_type).map_err(|e| CliError::new("INVALID_CONNECTION_TYPE", e))?;
    let port = flags.port.or_else(|| default_cli_port(db_type)).ok_or_else(|| {
        CliError::new("INVALID_ARGUMENT", "Port is required for this database type (use --port).")
    })?;
    let mut config = dbx_mcp::backend::new_connection_config(
        uuid::Uuid::new_v4().to_string(),
        name.to_string(),
        parsed,
        host.to_string(),
        port,
        flags.username.clone().unwrap_or_default(),
        flags.password.clone().unwrap_or_default(),
        flags.database.clone(),
        flags.ssl,
        flags.driver_profile.clone(),
    )
    .map_err(|e| CliError::new("INVALID_CONNECTION", e))?;

    let profile_ref = flags.proxy_profile_id.as_deref().is_some_and(|v| !v.trim().is_empty())
        || flags.proxy_profile_name.as_deref().is_some_and(|v| !v.trim().is_empty());
    let inline = flags.proxy
        || flags.proxy_host.as_deref().is_some_and(|v| !v.trim().is_empty())
        || flags.proxy_port.is_some();
    if profile_ref && inline {
        return Err(CliError::new(
            "PROXY_CONFLICT",
            "Cannot mix saved proxy reference with inline proxy settings.",
        ));
    }
    if profile_ref {
        config = dbx_mcp::resolve::apply_proxy_override_if_requested(
            backend,
            config,
            flags.proxy_profile_id.clone(),
            flags.proxy_profile_name.clone(),
        )
        .await
        .map_err(|error| {
            CliError::new(
                "PROXY_PROFILE_NOT_FOUND",
                error
                    .content
                    .first()
                    .and_then(|block| block.as_text())
                    .map(|text| text.text.clone())
                    .unwrap_or_else(|| "Proxy profile not found".into()),
            )
        })?;
    } else if flags.proxy || inline {
        let layer = dbx_mcp::tunnel_profiles::inline_proxy_layer(&dbx_mcp::tunnel_profiles::InlineProxyArgs {
            proxy_enabled: Some(true),
            proxy_host: flags.proxy_host.clone(),
            proxy_port: flags.proxy_port,
            proxy_username: flags.proxy_username.clone(),
            proxy_password: flags.proxy_password.clone(),
            proxy_type: flags.proxy_type.clone(),
        })
        .map_err(|e| CliError::new("INVALID_PROXY", e))?;
        config.transport_layers.push(layer);
    }

    let saved = backend.add_connection_for_mcp(config).await.map_err(store_error)?;
    if flags.format == OutputFormat::Json {
        return ok(format!(
            "{}\n",
            serde_json::to_string_pretty(&json!({ "id": saved.id, "name": saved.name })).unwrap()
        ));
    }
    ok(format!("Connection \"{}\" added (id: {}).\n", saved.name, saved.id))
}

fn default_cli_port(db_type: &str) -> Option<u16> {
    match db_type.to_ascii_lowercase().as_str() {
        "postgres" | "redshift" | "gaussdb" | "opengauss" | "kingbase" | "highgo" | "vastbase" => Some(5432),
        "mysql" | "doris" | "starrocks" | "mariadb" | "tidb" => Some(3306),
        "sqlserver" => Some(1433),
        "redis" => Some(6379),
        "mongodb" => Some(27017),
        "rqlite" => Some(4001),
        "sqlite" | "duckdb" | "access" => Some(0),
        _ => None,
    }
}

async fn find_connection(backend: &dyn DbxBackend, name: &str) -> Result<ConnectionConfig, CliError> {
    let connections = backend.load_connections().await.map_err(store_error)?;
    if let Ok(Some(indexes)) = dbx_mcp::list_index::parse_list_index_range(name) {
        if indexes.len() != 1 {
            return Err(CliError::new(
                "CONNECTION_RANGE",
                "This command accepts a single connection index; use a range-capable command (stats/report/query/redis/schema/context).",
            ));
        }
        let index = indexes[0];
        return connections.get(index - 1).cloned().ok_or_else(|| {
            CliError::new(
                "CONNECTION_NOT_FOUND",
                format!("List index #{index} is out of range (1-{}).", connections.len()),
            )
        });
    }
    let matching: Vec<_> = connections
        .into_iter()
        .filter(|connection| connection.name.eq_ignore_ascii_case(name) || connection.id == name)
        .collect();
    match matching.as_slice() {
        [] => Err(CliError::new("CONNECTION_NOT_FOUND", format!("Connection \"{name}\" not found."))),
        [connection] => Ok(connection.clone()),
        _ => Err(CliError::new(
            "AMBIGUOUS_CONNECTION",
            format!("Multiple connections match \"{name}\". Use a unique name or list index (#)."),
        )),
    }
}

fn selected_database(connection: &ConnectionConfig, override_database: Option<&str>) -> String {
    override_database.map(ToOwned::to_owned).or_else(|| connection.database.clone()).unwrap_or_default()
}

fn parse_flags(argv: &[String]) -> Result<Flags, CliError> {
    let mut flags = Flags {
        args: Vec::new(),
        format: OutputFormat::Table,
        schema: None,
        database: None,
        tables: Vec::new(),
        max_tables: None,
        max_rows: None,
        timeout_ms: None,
        file: None,
        allow_writes: false,
        allow_dangerous: false,
        help: false,
        version: false,
        quiet: false,
        verbose: false,
        parallel: None,
        skip_unsupported: true,
        no_save: false,
        output: None,
        name: None,
        db_type: None,
        host: None,
        port: None,
        username: None,
        password: None,
        ssl: false,
        driver_profile: None,
        proxy: false,
        proxy_type: None,
        proxy_host: None,
        proxy_port: None,
        proxy_username: None,
        proxy_password: None,
        proxy_profile_id: None,
        proxy_profile_name: None,
    };
    let mut index = 0;
    while index < argv.len() {
        let arg = &argv[index];
        if arg == "--" {
            flags.args.extend(argv[index + 1..].iter().cloned());
            break;
        }
        match arg.as_str() {
            "--json" | "-j" => flags.format = OutputFormat::Json,
            "--format" => {
                let value = option_value(argv, &mut index, "--format")?;
                flags.format = match value.as_str() {
                    "table" => OutputFormat::Table,
                    "json" => OutputFormat::Json,
                    "csv" => OutputFormat::Csv,
                    _ => return Err(CliError::new("INVALID_OPTION", "--format must be one of: table, json, csv.")),
                };
            }
            "--help" | "-h" => flags.help = true,
            "--version" | "-V" => flags.version = true,
            "--quiet" | "-q" => flags.quiet = true,
            "--verbose" | "-v" => flags.verbose = true,
            "--parallel" | "-P" => {
                let next = argv.get(index + 1);
                if let Some(value) = next.filter(|value| !value.starts_with('-') && value.chars().all(|c| c.is_ascii_digit()))
                {
                    let n = value.parse::<usize>().unwrap_or(0);
                    flags.parallel = Some(if n == 0 {
                        dbx_mcp::list_index::DEFAULT_PARALLEL_CONCURRENCY
                    } else {
                        n
                    });
                    index += 1;
                } else {
                    flags.parallel = Some(dbx_mcp::list_index::DEFAULT_PARALLEL_CONCURRENCY);
                }
            }
            "--schema" | "-s" => flags.schema = Some(option_value(argv, &mut index, "--schema")?),
            "--database" | "-d" => flags.database = Some(option_value(argv, &mut index, "--database")?),
            "--tables" => {
                flags.tables = option_value(argv, &mut index, "--tables")?
                    .split(',')
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(ToOwned::to_owned)
                    .collect()
            }
            "--max-tables" => {
                flags.max_tables =
                    Some(positive_usize(&option_value(argv, &mut index, "--max-tables")?, "--max-tables")?)
            }
            "--limit" => flags.max_rows = Some(positive_usize(&option_value(argv, &mut index, "--limit")?, "--limit")?),
            "--timeout" | "-t" => {
                flags.timeout_ms = Some(duration_ms(&option_value(argv, &mut index, "--timeout")?, "--timeout")?)
            }
            "--file" => flags.file = Some(PathBuf::from(option_value(argv, &mut index, "--file")?)),
            "--allow-writes" => flags.allow_writes = true,
            "--allow-dangerous-sql" => flags.allow_dangerous = true,
            "--skip-unsupported" => flags.skip_unsupported = true,
            "--no-skip-unsupported" => flags.skip_unsupported = false,
            "--no-save" | "-n" => flags.no_save = true,
            "--output" | "-o" => flags.output = Some(PathBuf::from(option_value(argv, &mut index, "--output")?)),
            "--name" => flags.name = Some(option_value(argv, &mut index, "--name")?),
            "--type" => flags.db_type = Some(option_value(argv, &mut index, "--type")?),
            "--host" => flags.host = Some(option_value(argv, &mut index, "--host")?),
            "--port" => {
                flags.port = Some(
                    option_value(argv, &mut index, "--port")?
                        .parse()
                        .map_err(|_| CliError::new("INVALID_OPTION", "--port must be a number."))?,
                )
            }
            "--username" => flags.username = Some(option_value(argv, &mut index, "--username")?),
            "--password" => flags.password = Some(option_value(argv, &mut index, "--password")?),
            "--ssl" => flags.ssl = true,
            "--driver-profile" => flags.driver_profile = Some(option_value(argv, &mut index, "--driver-profile")?),
            "--proxy" => flags.proxy = true,
            "--proxy-type" => flags.proxy_type = Some(option_value(argv, &mut index, "--proxy-type")?),
            "--proxy-host" | "-H" => flags.proxy_host = Some(option_value(argv, &mut index, "--proxy-host")?),
            "--proxy-port" => {
                flags.proxy_port = Some(
                    option_value(argv, &mut index, "--proxy-port")?
                        .parse()
                        .map_err(|_| CliError::new("INVALID_OPTION", "--proxy-port must be a number."))?,
                )
            }
            "--proxy-username" => flags.proxy_username = Some(option_value(argv, &mut index, "--proxy-username")?),
            "--proxy-password" => flags.proxy_password = Some(option_value(argv, &mut index, "--proxy-password")?),
            "--proxy-profile-id" => flags.proxy_profile_id = Some(option_value(argv, &mut index, "--proxy-profile-id")?),
            "--proxy-profile-name" => {
                flags.proxy_profile_name = Some(option_value(argv, &mut index, "--proxy-profile-name")?)
            }
            value if value.starts_with('-') => {
                return Err(CliError::new("UNKNOWN_OPTION", format!("Unknown option: {value}")))
            }
            _ => flags.args.push(arg.clone()),
        }
        index += 1;
    }
    Ok(flags)
}

fn option_value(argv: &[String], index: &mut usize, option: &'static str) -> Result<String, CliError> {
    *index += 1;
    argv.get(*index)
        .filter(|value| !value.starts_with('-'))
        .cloned()
        .ok_or_else(|| CliError::new("INVALID_OPTION", format!("{option} requires a value.")))
}

fn positive_usize(value: &str, option: &'static str) -> Result<usize, CliError> {
    value
        .parse::<usize>()
        .ok()
        .filter(|value| *value > 0)
        .ok_or_else(|| CliError::new("INVALID_OPTION", format!("{option} must be a positive integer.")))
}

fn duration_ms(value: &str, option: &'static str) -> Result<u64, CliError> {
    let (number, multiplier) = if let Some(value) = value.strip_suffix("ms") {
        (value, 1)
    } else if let Some(value) = value.strip_suffix('s') {
        (value, 1000)
    } else if let Some(value) = value.strip_suffix('m') {
        (value, 60_000)
    } else {
        (value, 1)
    };
    number
        .parse::<u64>()
        .ok()
        .filter(|amount| *amount > 0)
        .and_then(|amount| amount.checked_mul(multiplier))
        .ok_or_else(|| {
            CliError::new("INVALID_OPTION", format!("{option} must be a positive duration such as 500ms, 10s, or 1m."))
        })
}

fn ensure_arg_count(args: &[String], count: usize, command: &'static str) -> Result<(), CliError> {
    if args.len() == count {
        Ok(())
    } else {
        Err(CliError::new(
            "INVALID_ARGUMENT",
            format!("{command} expects {} argument(s); received {}.", count - 1, args.len().saturating_sub(1)),
        ))
    }
}

fn required<'a>(value: Option<&'a String>, message: &'static str) -> Result<&'a str, CliError> {
    value.map(String::as_str).filter(|value| !value.is_empty()).ok_or_else(|| CliError::new("ERROR", message))
}

fn env_flag(name: &str) -> bool {
    env::var(name).ok().is_some_and(|value| matches!(value.trim().to_ascii_lowercase().as_str(), "1" | "true"))
}

fn store_error(message: String) -> CliError {
    CliError::new("CONNECTION_STORE_ERROR", message)
}
fn command_error(message: String) -> CliError {
    CliError::new("ERROR", message)
}

fn db_type_name(db_type: DatabaseType) -> String {
    serde_json::to_value(db_type)
        .ok()
        .and_then(|value| value.as_str().map(ToOwned::to_owned))
        .unwrap_or_else(|| format!("{db_type:?}").to_ascii_lowercase())
}

fn format_connections(connections: &[ConnectionConfig], format: OutputFormat) -> Result<String, CliError> {
    let rows: Vec<Value> = connections
        .iter()
        .enumerate()
        .map(|(idx, connection)| {
            optional_object([
                ("index", Some(json!(idx + 1))),
                ("id", Some(json!(connection.id))),
                ("name", Some(json!(connection.name))),
                ("type", Some(json!(db_type_name(connection.db_type)))),
                ("host", Some(json!(connection.host))),
                ("port", Some(json!(connection.port))),
                ("database", connection.database.clone().filter(|value| !value.is_empty()).map(|value| json!(value))),
            ])
        })
        .collect();
    match format {
        OutputFormat::Json => json_string(&json!({ "connections": rows })),
        OutputFormat::Csv => Ok(csv_table(&["index", "id", "name", "type", "host", "port", "database"], &rows)),
        OutputFormat::Table => Ok(format!(
            "{}\n",
            markdown_table(
                &["#", "ID", "Name", "Type", "Host", "Port", "Database"],
                &rows,
                &["index", "id", "name", "type", "host", "port", "database"]
            )
        )),
    }
}

fn format_tables(
    connection: &str,
    schema: Option<&str>,
    tables: &[TableInfo],
    format: OutputFormat,
) -> Result<String, CliError> {
    let rows: Vec<Value> = tables
        .iter()
        .map(|table| optional_object([("name", Some(json!(table.name))), ("type", Some(json!(table.table_type)))]))
        .collect();
    match format {
        OutputFormat::Json => json_string(&optional_object([
            ("connection", Some(json!(connection))),
            ("schema", schema.map(|value| json!(value))),
            ("tables", Some(json!(rows))),
        ])),
        OutputFormat::Csv => Ok(csv_table(&["name", "type"], &rows)),
        OutputFormat::Table => Ok(format!("{}\n", markdown_table(&["Table", "Type"], &rows, &["name", "type"]))),
    }
}

fn format_columns(
    connection: &str,
    schema: Option<&str>,
    table: &str,
    columns: &[ColumnInfo],
    format: OutputFormat,
) -> Result<String, CliError> {
    if format == OutputFormat::Json {
        return json_string(&optional_object([
            ("connection", Some(json!(connection))),
            ("schema", schema.map(|value| json!(value))),
            ("table", Some(json!(table))),
            ("columns", Some(json!(columns))),
        ]));
    }
    let rows: Vec<Value> = columns.iter().map(|column| json!({ "name": column.name, "data_type": column.data_type, "is_nullable": column.is_nullable, "is_primary_key": column.is_primary_key, "column_default": column.column_default, "comment": column.comment, "display_name": if column.is_primary_key { format!("{} (PK)", column.name) } else { column.name.clone() }, "nullable": if column.is_nullable { "YES" } else { "NO" } })).collect();
    if format == OutputFormat::Csv {
        return Ok(csv_table(
            &["name", "data_type", "is_nullable", "is_primary_key", "column_default", "comment"],
            &rows,
        ));
    }
    Ok(format!(
        "{}\n",
        markdown_table(
            &["Column", "Type", "Nullable", "Default", "Comment"],
            &rows,
            &["display_name", "data_type", "nullable", "column_default", "comment"]
        )
    ))
}

fn format_query(connection: &str, result: &QueryResult, format: OutputFormat) -> Result<String, CliError> {
    let rows: Vec<Value> = result
        .rows
        .iter()
        .map(|values| {
            Value::Object(result.columns.iter().cloned().zip(values.iter().cloned()).collect::<Map<String, Value>>())
        })
        .collect();
    let row_count = if result.columns.is_empty() { result.affected_rows } else { result.rows.len() as u64 };
    match format {
        OutputFormat::Json => json_string(
            &json!({ "connection": connection, "columns": result.columns, "rows": rows, "row_count": row_count }),
        ),
        OutputFormat::Csv => Ok(csv_table(&result.columns.iter().map(String::as_str).collect::<Vec<_>>(), &rows)),
        OutputFormat::Table if result.columns.is_empty() => {
            Ok(format!("Query executed. {row_count} row(s) affected.\n"))
        }
        OutputFormat::Table => Ok(format!(
            "{}\n\n{row_count} row(s)\n",
            markdown_table(
                &result.columns.iter().map(String::as_str).collect::<Vec<_>>(),
                &rows,
                &result.columns.iter().map(String::as_str).collect::<Vec<_>>()
            )
        )),
    }
}

fn format_capabilities(format: OutputFormat) -> Result<String, CliError> {
    match format {
        OutputFormat::Json => json_string(
            &json!({ "directQueryTypes": DIRECT_QUERY_TYPES, "bridgeRequiredTypes": BRIDGE_REQUIRED_TYPES }),
        ),
        OutputFormat::Csv => {
            let rows: Vec<Value> = DIRECT_QUERY_TYPES
                .iter()
                .map(|kind| json!({ "mode": "direct", "type": kind }))
                .chain(BRIDGE_REQUIRED_TYPES.iter().map(|kind| json!({ "mode": "bridge", "type": kind })))
                .collect();
            Ok(csv_table(&["mode", "type"], &rows))
        }
        OutputFormat::Table => {
            let rows = vec![
                json!({ "mode": "Direct", "types": DIRECT_QUERY_TYPES.join(", ") }),
                json!({ "mode": "Requires DBX Desktop", "types": BRIDGE_REQUIRED_TYPES.join(", ") }),
            ];
            Ok(format!("{}\n", markdown_table(&["Mode", "Types"], &rows, &["mode", "types"])))
        }
    }
}

async fn diagnostics() -> Diagnostics {
    let app_data_dir = dbx_mcp::paths::app_data_dir().unwrap_or_default();
    let db_path = app_data_dir.join(dbx_mcp::paths::STORAGE_DB_FILE_NAME);
    let bridge_port_file = app_data_dir.join("mcp-bridge-port");
    let db_path_exists = db_path.exists();
    let bridge_port_file_exists = bridge_port_file.exists();
    let bridge_url = if bridge_port_file_exists {
        tokio::fs::read_to_string(&bridge_port_file).await.ok().map(|port| format!("http://127.0.0.1:{}", port.trim()))
    } else {
        None
    };
    let loaded = if db_path_exists {
        match LocalBackend::open(&db_path).await {
            Ok(backend) => backend.load_connections().await,
            Err(error) => Err(error),
        }
    } else {
        Err("DBX database does not exist.".to_string())
    };
    let (load_connections_ok, connections, error) = match loaded {
        Ok(connections) => (true, connections, None),
        Err(error) => (false, Vec::new(), Some(error)),
    };
    Diagnostics {
        app_data_dir: app_data_dir.display().to_string(),
        db_path: db_path.display().to_string(),
        db_path_exists,
        connections_table_exists: load_connections_ok,
        connection_row_count: connections.len(),
        load_connections_ok,
        loaded_connection_count: connections.len(),
        load_connections_error: error,
        load_connections_hint: None,
        bridge_port_file: bridge_port_file.display().to_string(),
        bridge_port_file_exists,
        bridge_url,
        direct_query_types: DIRECT_QUERY_TYPES.to_vec(),
        bridge_required_types: BRIDGE_REQUIRED_TYPES.to_vec(),
    }
}

fn format_diagnostics(value: &Diagnostics, format: OutputFormat) -> Result<String, CliError> {
    if format == OutputFormat::Json {
        return json_string(value);
    }
    let rows = vec![
        json!({ "check": "App data directory", "value": value.app_data_dir }),
        json!({ "check": "DBX database", "value": if value.db_path_exists { format!("found ({})", value.db_path) } else { format!("missing ({})", value.db_path) } }),
        json!({ "check": "Connections table", "value": if value.connections_table_exists { format!("{} row(s)", value.connection_row_count) } else { "missing".to_string() } }),
        json!({ "check": "Connection loading", "value": if value.load_connections_ok { format!("ok ({} loaded)", value.loaded_connection_count) } else { format!("failed ({})", value.load_connections_error.as_deref().unwrap_or("unknown error")) } }),
        json!({ "check": "Desktop bridge", "value": if value.bridge_port_file_exists { format!("available ({})", value.bridge_url.as_deref().unwrap_or(&value.bridge_port_file)) } else { "not running".to_string() } }),
        json!({ "check": "Direct query types", "value": value.direct_query_types.join(", ") }),
        json!({ "check": "Bridge-required types", "value": value.bridge_required_types.join(", ") }),
    ];
    if format == OutputFormat::Csv {
        return Ok(csv_table(&["check", "value"], &rows));
    }
    Ok(format!("{}\n", markdown_table(&["Check", "Value"], &rows, &["check", "value"])))
}

fn json_string(value: &impl Serialize) -> Result<String, CliError> {
    serde_json::to_string_pretty(value)
        .map(|value| format!("{value}\n"))
        .map_err(|error| CliError::new("ERROR", error.to_string()))
}

fn optional_object<const N: usize>(fields: [(&str, Option<Value>); N]) -> Value {
    let mut object = Map::new();
    for (key, value) in fields {
        if let Some(value) = value {
            object.insert(key.to_string(), value);
        }
    }
    Value::Object(object)
}

fn markdown_table(headers: &[&str], rows: &[Value], keys: &[&str]) -> String {
    let mut output = format!("| {} |\n| {} |", headers.join(" | "), vec!["---"; headers.len()].join(" | "));
    for row in rows {
        output.push_str(&format!(
            "\n| {} |",
            keys.iter().map(|key| format_cell(&row[*key])).collect::<Vec<_>>().join(" | ")
        ));
    }
    output
}

fn csv_table(headers: &[&str], rows: &[Value]) -> String {
    let mut output = format!("{}\n", headers.join(","));
    for row in rows {
        output.push_str(&format!(
            "{}\n",
            headers.iter().map(|key| csv_cell(&format_cell(&row[*key]))).collect::<Vec<_>>().join(",")
        ));
    }
    output
}

fn format_cell(value: &Value) -> String {
    match value {
        Value::Null => String::new(),
        Value::String(value) => value.replace('|', "\\|").replace('\n', " "),
        Value::Bool(value) => value.to_string(),
        other => other.to_string(),
    }
}

fn csv_cell(value: &str) -> String {
    if value.contains([',', '"', '\n']) {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.to_string()
    }
}

fn usage() -> &'static str {
    "Usage:\n  dbx doctor [-j, --json]\n  dbx capabilities [-j, --json]\n  dbx connections list [-j, --json]\n  dbx connections add --name <name> --type <db_type> --host <host> [--port n] [--username u] [--password p] [-d, --database db] [--ssl] [--driver-profile x]\n      [--proxy] [--proxy-type socks5|http] [-H, --proxy-host h] [--proxy-port n] [--proxy-username u] [--proxy-password p]\n      [--proxy-profile-id id|# | --proxy-profile-name name|#] [-j, --json]\n  dbx connections remove <connection|#> [-j, --json]\n  dbx proxies list [-j, --json]\n  dbx stats <connection|#|range> [-s, --schema name] [-d, --database name] [-t, --timeout 60s] [-P, --parallel [n]] [--skip-unsupported|--no-skip-unsupported] [-q, --quiet] [-v, --verbose] [-j, --json]\n  dbx report <connection|#|range> [-s, --schema name] [-d, --database name] [-t, --timeout 60s] [-P, --parallel [n]] [--skip-unsupported|--no-skip-unsupported] [-q, --quiet] [-v, --verbose] [-j, --json] [-n, --no-save] [-o, --output path]\n  dbx schema list <connection|#|range> [-s, --schema name] [-P, --parallel [n]] [-q, --quiet] [-v, --verbose] [-j, --json]\n  dbx schema describe <connection|#|range> <table> [-s, --schema name] [-P, --parallel [n]] [-q, --quiet] [-v, --verbose] [-j, --json]\n  dbx query <connection|#|range> <sql> [--file path] [--limit n] [-t, --timeout 10s] [--allow-writes] [--allow-dangerous-sql] [-P, --parallel [n]] [--proxy-profile-id id|# | --proxy-profile-name name|#] [-q, --quiet] [-v, --verbose] [-j, --json]\n  dbx redis <connection|#|range> <command...> [-d, --database n] [-t, --timeout 10s] [--allow-writes] [--allow-dangerous-sql] [-P, --parallel [n]] [-q, --quiet] [-v, --verbose] [-j, --json]\n  dbx context <connection|#|range> [-s, --schema name] [--tables a,b] [--max-tables n] [-P, --parallel [n]] [-q, --quiet] [-v, --verbose] [-j, --json]\n  dbx open <connection|#|range> <table> [-s, --schema name] [-d, --database name] [-P, --parallel [n]] [-j, --json]\n\nOptions:\n  -j, --json           JSON output\n  -q, --quiet          Suppress progress on stderr\n  -v, --verbose        Extra progress detail (e.g. SQL text)\n  -P, --parallel [n]   Concurrent batch (default concurrency 15)\n  -d, --database NAME  Target database\n  -s, --schema NAME    Target schema\n  -t, --timeout DUR    Query timeout (e.g. 500ms, 60s, 1m)\n  -H, --proxy-host H   Proxy host (connections add)\n  -o, --output PATH    Report output file or batch directory\n  -n, --no-save        Skip saving report to file\n  --skip-unsupported   stats/report: treat unsupported types as skipped (default)\n  --no-skip-unsupported stats/report: treat unsupported types as failures"
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use dbx_core::{
        agent_events::ToolResult,
        agent_tools::AgentSqlPermissions,
        storage::{McpGlobalPolicy, Storage},
    };
    use dbx_mcp::{backend::new_connection_config, mongo::MongoCommand};

    struct MongoBackend {
        connection: ConnectionConfig,
    }

    impl MongoBackend {
        fn new() -> Self {
            Self {
                connection: new_connection_config(
                    "mongo-test".to_string(),
                    "local-mongo".to_string(),
                    DatabaseType::MongoDb,
                    "127.0.0.1".to_string(),
                    27017,
                    String::new(),
                    String::new(),
                    Some("test".to_string()),
                    false,
                    None,
                )
                .unwrap(),
            }
        }
    }

    #[async_trait]
    impl DbxBackend for MongoBackend {
        async fn load_mcp_global_policy(&self) -> Result<McpGlobalPolicy, String> {
            Ok(McpGlobalPolicy::default())
        }

        async fn load_connections(&self) -> Result<Vec<ConnectionConfig>, String> {
            Ok(vec![self.connection.clone()])
        }

        async fn execute_agent_tool(
            &self,
            _connection: &ConnectionConfig,
            _database: &str,
            _tool_name: &str,
            _arguments: Value,
            _permissions: AgentSqlPermissions,
        ) -> ToolResult {
            panic!("Mongo CLI queries must not fall through to agent SQL execution")
        }

        async fn execute_mongo_command(
            &self,
            _connection: &ConnectionConfig,
            _database: &str,
            command: &MongoCommand,
        ) -> Result<QueryResult, String> {
            assert!(matches!(command, MongoCommand::Insert { collection, .. } if collection == "products"));
            Ok(QueryResult {
                columns: Vec::new(),
                column_types: Vec::new(),
                column_sortables: Vec::new(),
                rows: Vec::new(),
                affected_rows: 2,
                execution_time_ms: 0,
                truncated: false,
                session_id: None,
                has_more: false,
            })
        }

        async fn add_connection_for_mcp(&self, config: ConnectionConfig) -> Result<ConnectionConfig, String> {
            Ok(config)
        }

        async fn remove_connection_for_mcp(&self, _connection_id: &str) -> Result<bool, String> {
            Ok(true)
        }
    }

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_string()).collect()
    }

    #[test]
    fn parses_existing_json_and_query_flags() {
        let flags =
            parse_flags(&args(&["query", "local", "select 1", "--limit", "50", "--timeout", "10s", "--json"])).unwrap();
        assert_eq!(flags.args, args(&["query", "local", "select 1"]));
        assert_eq!(flags.max_rows, Some(50));
        assert_eq!(flags.timeout_ms, Some(10_000));
        assert!(flags.format == OutputFormat::Json);
    }

    #[test]
    fn parses_short_flags_and_parallel() {
        let flags = parse_flags(&args(&["stats", "1-3", "-j", "-q", "-P", "3", "-t", "60s", "-d", "db"])).unwrap();
        assert_eq!(flags.args, args(&["stats", "1-3"]));
        assert!(flags.format == OutputFormat::Json);
        assert!(flags.quiet);
        assert_eq!(flags.parallel, Some(3));
        assert_eq!(flags.timeout_ms, Some(60_000));
        assert_eq!(flags.database.as_deref(), Some("db"));
    }

    #[test]
    fn preserves_double_dash_sql() {
        let flags = parse_flags(&args(&["query", "local", "--json", "--", "-- comment\nselect 1"])).unwrap();
        assert_eq!(flags.args, args(&["query", "local", "-- comment\nselect 1"]));
    }

    #[test]
    fn rejects_unknown_options_with_stable_code() {
        let error = parse_flags(&args(&["connections", "list", "--wat"])).unwrap_err();
        assert_eq!(error.code, "UNKNOWN_OPTION");
    }

    #[test]
    fn formats_csv_using_existing_escaping_rules() {
        let rows = vec![json!({ "name": "alpha,beta", "value": "a\"b" })];
        assert_eq!(csv_table(&["name", "value"], &rows), "name,value\n\"alpha,beta\",\"a\"\"b\"\n");
    }

    #[test]
    fn dangerous_sql_requires_explicit_permission() {
        let risk = classify_sql_risk_for_database("drop table users", DatabaseType::Postgres).unwrap();
        assert_eq!(risk, SqlRisk::Ddl);
    }

    #[tokio::test]
    async fn routes_legacy_mongo_insert_through_shared_mongo_backend() {
        let flags = parse_flags(&args(&[
            "query",
            "local-mongo",
            "db.products.insert([{name: 'first'}, {name: 'second'}])",
            "--allow-writes",
            "--json",
        ]))
        .unwrap();
        let output = outcome_text(run_with_backend(Arc::new(MongoBackend::new()), flags).await.unwrap());
        let value: Value = serde_json::from_str(&output).unwrap();
        assert_eq!(value["connection"], "local-mongo");
        assert_eq!(value["row_count"], 2);
        assert_eq!(value["columns"], json!([]));
    }

    #[tokio::test]
    async fn blocks_mongo_writes_without_explicit_permission() {
        let flags =
            parse_flags(&args(&["query", "local-mongo", "db.products.insertOne({name: 'demo'})", "--json"])).unwrap();
        let error = run_with_backend(Arc::new(MongoBackend::new()), flags).await.unwrap_err();
        assert_eq!(error.code, "SQL_BLOCKED");
    }

    #[tokio::test]
    #[ignore = "requires DBX_MCP_TEST_MONGO_HOST and DBX_MCP_TEST_MONGO_PASSWORD"]
    async fn executes_legacy_mongo_insert_without_desktop_process() {
        let host = env::var("DBX_MCP_TEST_MONGO_HOST").expect("MongoDB host");
        let port = env::var("DBX_MCP_TEST_MONGO_PORT")
            .unwrap_or_else(|_| "27017".to_string())
            .parse::<u16>()
            .expect("MongoDB port");
        let password = env::var("DBX_MCP_TEST_MONGO_PASSWORD").expect("MongoDB password");
        let directory = tempfile::tempdir().expect("temporary data directory");
        let db_path = directory.path().join("dbx.db");
        let storage = Storage::open(&db_path).await.expect("open storage");
        let mut connection = new_connection_config(
            "mongo-cli-e2e".to_string(),
            "mongo-cli-e2e".to_string(),
            DatabaseType::MongoDb,
            host,
            port,
            "root".to_string(),
            password,
            Some("dbx_mcp_test".to_string()),
            false,
            None,
        )
        .unwrap();
        connection.url_params = Some("authSource=admin".to_string());
        storage.save_connections(&[connection]).await.expect("save connection");
        let backend: Arc<dyn DbxBackend> = Arc::new(LocalBackend::open(&db_path).await.expect("open local backend"));

        let cleanup = parse_flags(&args(&[
            "query",
            "mongo-cli-e2e",
            "db.items.deleteMany({_id: {$in: ['rust-cli-e2e-1', 'rust-cli-e2e-2']}})",
            "--allow-writes",
        ]))
        .unwrap();
        run_with_backend(Arc::clone(&backend), cleanup).await.expect("initial cleanup");

        let insert = parse_flags(&args(&[
            "query",
            "mongo-cli-e2e",
            "db.items.insert([{_id: 'rust-cli-e2e-1', name: 'Ada'}, {_id: 'rust-cli-e2e-2', name: 'Grace'}])",
            "--allow-writes",
            "--json",
        ]))
        .unwrap();
        let inserted: Value = serde_json::from_str(&outcome_text(run_with_backend(Arc::clone(&backend), insert).await.unwrap())).unwrap();
        assert_eq!(inserted["row_count"], 2);

        let find = parse_flags(&args(&[
            "query",
            "mongo-cli-e2e",
            "db.items.find({_id: {$in: ['rust-cli-e2e-1', 'rust-cli-e2e-2']}}).sort({_id: 1})",
            "--json",
        ]))
        .unwrap();
        let found: Value = serde_json::from_str(&outcome_text(run_with_backend(Arc::clone(&backend), find).await.unwrap())).unwrap();
        assert_eq!(found["row_count"], 2);
        assert_eq!(found["rows"][0]["name"], "Ada");
        assert_eq!(found["rows"][1]["name"], "Grace");

        let cleanup = parse_flags(&args(&[
            "query",
            "mongo-cli-e2e",
            "db.items.deleteMany({_id: {$in: ['rust-cli-e2e-1', 'rust-cli-e2e-2']}})",
            "--allow-writes",
        ]))
        .unwrap();
        run_with_backend(Arc::clone(&backend), cleanup).await.expect("final cleanup");
    }
}
