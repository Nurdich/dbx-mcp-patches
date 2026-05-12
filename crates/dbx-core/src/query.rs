use std::future::Future;
use std::time::Duration;
use tokio::time::timeout;
use tokio_util::sync::CancellationToken;

use crate::connection::{AppState, PoolKind};
use crate::db;
use crate::sql::{split_sql_batches, split_sql_statements, starts_with_executable_sql_keyword};

pub const QUERY_TIMEOUT: Duration = Duration::from_secs(30);
pub const MAX_ROWS: usize = 10000;
pub const QUERY_CANCELED: &str = "Query canceled";

pub fn duckdb_execute(con: &duckdb::Connection, sql: &str) -> Result<db::QueryResult, String> {
    duckdb_execute_with_row_limit(con, sql, MAX_ROWS)
}

pub fn duckdb_execute_with_row_limit(
    con: &duckdb::Connection,
    sql: &str,
    row_limit: usize,
) -> Result<db::QueryResult, String> {
    let start = std::time::Instant::now();
    let row_limit = row_limit.max(1);

    if starts_with_executable_sql_keyword(sql, &["SELECT", "SHOW", "DESCRIBE", "EXPLAIN", "WITH", "PRAGMA"]) {
        let mut stmt = con.prepare(sql).map_err(|e| e.to_string())?;
        let mut rows = stmt.query([]).map_err(|e| e.to_string())?;
        let stmt_ref = rows.as_ref().ok_or("DuckDB statement unavailable")?;
        let col_count = stmt_ref.column_count();
        let columns: Vec<String> = (0..col_count)
            .map(|i| stmt_ref.column_name(i).map(|s| s.to_string()).unwrap_or_else(|_| "?".to_string()))
            .collect();

        let mut result_rows = Vec::new();
        while let Some(row) = rows.next().map_err(|e| e.to_string())? {
            let vals: Vec<serde_json::Value> = (0..col_count)
                .map(|i| {
                    row.get::<_, String>(i)
                        .map(serde_json::Value::String)
                        .or_else(|_| row.get::<_, i64>(i).map(|v| serde_json::Value::Number(v.into())))
                        .or_else(|_| {
                            row.get::<_, f64>(i).map(|v| {
                                serde_json::Number::from_f64(v)
                                    .map(serde_json::Value::Number)
                                    .unwrap_or(serde_json::Value::Null)
                            })
                        })
                        .or_else(|_| row.get::<_, bool>(i).map(serde_json::Value::Bool))
                        .unwrap_or(serde_json::Value::Null)
                })
                .collect();
            result_rows.push(vals);
            if result_rows.len() > row_limit {
                break;
            }
        }

        let truncated = result_rows.len() > row_limit;
        if truncated {
            result_rows.truncate(row_limit);
        }
        Ok(db::QueryResult {
            columns,
            rows: result_rows,
            affected_rows: 0,
            execution_time_ms: start.elapsed().as_millis(),
            truncated,
        })
    } else {
        let affected = con.execute(sql, []).map_err(|e| e.to_string())?;
        Ok(db::QueryResult {
            columns: vec![],
            rows: vec![],
            affected_rows: affected as u64,
            execution_time_ms: start.elapsed().as_millis(),
            truncated: false,
        })
    }
}

pub fn truncate_result(result: db::QueryResult) -> db::QueryResult {
    truncate_result_with_row_limit(result, MAX_ROWS)
}

pub fn truncate_result_with_row_limit(mut result: db::QueryResult, row_limit: usize) -> db::QueryResult {
    let row_limit = row_limit.max(1);
    if result.rows.len() > row_limit {
        result.rows.truncate(row_limit);
        result.truncated = true;
    }
    result
}

pub fn is_connection_error(err: &str) -> bool {
    let lower = err.to_lowercase();
    lower.contains("connection")
        || lower.contains("broken pipe")
        || lower.contains("reset by peer")
        || lower.contains("timed out")
        || lower.contains("closed")
        || lower.contains("eof")
        || lower.contains("i/o error")
        || is_os_connection_error(&lower)
}

fn is_os_connection_error(lower: &str) -> bool {
    let os_error_codes = ["10053", "10054", "10057", "10058", "10060", "10061"];
    if let Some(pos) = lower.find("os error ") {
        let after = &lower[pos + 9..];
        let code: String = after.chars().take_while(|c| c.is_ascii_digit()).collect();
        return os_error_codes.contains(&code.as_str());
    }
    false
}

pub fn timeout_error() -> String {
    format!("Query timed out after {} seconds", QUERY_TIMEOUT.as_secs())
}

pub fn canceled_error() -> String {
    QUERY_CANCELED.to_string()
}

pub fn is_canceled(cancel_token: &Option<CancellationToken>) -> bool {
    cancel_token.as_ref().map(|token| token.is_cancelled()).unwrap_or(false)
}

pub async fn wait_for_query<F>(cancel_token: Option<CancellationToken>, future: F) -> Result<db::QueryResult, String>
where
    F: Future<Output = Result<db::QueryResult, String>>,
{
    wait_for_query_with_timeout(cancel_token, QUERY_TIMEOUT, future).await
}

pub async fn wait_for_query_with_timeout<F>(
    cancel_token: Option<CancellationToken>,
    timeout_duration: Duration,
    future: F,
) -> Result<db::QueryResult, String>
where
    F: Future<Output = Result<db::QueryResult, String>>,
{
    if let Some(token) = cancel_token {
        tokio::select! {
            biased;
            _ = token.cancelled() => Err(canceled_error()),
            result = timeout(timeout_duration, future) => result.map_err(|_| timeout_error())?,
        }
    } else {
        timeout(timeout_duration, future).await.map_err(|_| timeout_error())?
    }
}

pub async fn do_execute(
    state: &AppState,
    pool_key: &str,
    sql: &str,
    schema: Option<&str>,
    cancel_token: Option<CancellationToken>,
) -> Result<db::QueryResult, String> {
    do_execute_with_row_limit(state, pool_key, sql, schema, cancel_token, MAX_ROWS).await
}

pub async fn do_execute_with_row_limit(
    state: &AppState,
    pool_key: &str,
    sql: &str,
    schema: Option<&str>,
    cancel_token: Option<CancellationToken>,
    row_limit: usize,
) -> Result<db::QueryResult, String> {
    let row_limit = row_limit.max(1);
    let connections = state.connections.read().await;
    let pool = connections.get(pool_key).ok_or("Connection not found")?;

    match pool {
        PoolKind::DuckDb(con) => {
            let con = con.clone();
            let sql = sql.to_string();
            drop(connections);
            wait_for_query(cancel_token, async move {
                let task = tokio::task::spawn_blocking(move || {
                    let con = con.lock().map_err(|e| e.to_string())?;
                    duckdb_execute_with_row_limit(&con, &sql, row_limit)
                });
                task.await.map_err(|e| e.to_string())?
            })
            .await
        }
        PoolKind::Mysql(p, mode) => {
            let p = p.clone();
            let bare = *mode == crate::connection::MysqlMode::Bare;
            drop(connections);
            wait_for_query(cancel_token, db::mysql::execute_query_with_row_limit(&p, sql, bare, row_limit)).await
        }
        PoolKind::Postgres(p) => {
            let p = p.clone();
            let schema = schema.map(|s| s.to_string());
            drop(connections);
            if let Some(schema) = schema {
                wait_for_query(
                    cancel_token,
                    db::postgres::execute_query_with_schema_and_row_limit(&p, &schema, sql, row_limit),
                )
                .await
            } else {
                wait_for_query(cancel_token, db::postgres::execute_query_with_row_limit(&p, sql, row_limit)).await
            }
        }
        PoolKind::Sqlite(p) => {
            let p = p.clone();
            drop(connections);
            wait_for_query(cancel_token, db::sqlite::execute_query_with_row_limit(&p, sql, row_limit)).await
        }
        PoolKind::ClickHouse(client) => {
            let client = client.clone();
            let database = pool_key.split(':').nth(1).unwrap_or("default").to_string();
            drop(connections);
            wait_for_query(cancel_token, db::clickhouse_driver::execute_query(&client, &database, sql))
                .await
                .map(|result| truncate_result_with_row_limit(result, row_limit))
        }
        PoolKind::SqlServer(client) => {
            let client = client.clone();
            drop(connections);
            let mut client = match cancel_token.as_ref() {
                Some(token) => tokio::select! {
                    biased;
                    _ = token.cancelled() => return Err(canceled_error()),
                    guard = client.lock() => guard,
                },
                None => client.lock().await,
            };
            wait_for_query(cancel_token, db::sqlserver::execute_query(&mut client, sql))
                .await
                .map(|result| truncate_result_with_row_limit(result, row_limit))
        }
        PoolKind::Oracle(pool) => {
            let client = pool.client();
            let schema = schema.map(|s| s.to_string());
            drop(connections);
            log::info!("[query][oracle:lock:start] schema={:?} sql={}", schema, sql);
            let client = match cancel_token.as_ref() {
                Some(token) => tokio::select! {
                    biased;
                    _ = token.cancelled() => return Err(canceled_error()),
                    guard = client.lock() => guard,
                },
                None => client.lock().await,
            };
            log::info!("[query][oracle:lock:done] schema={:?}", schema);
            if let Some(schema) = schema {
                wait_for_query(cancel_token, db::oracle_driver::execute_query_with_schema(&*client, &schema, sql))
                    .await
                    .map(|result| truncate_result_with_row_limit(result, row_limit))
            } else {
                wait_for_query(cancel_token, db::oracle_driver::execute_query(&*client, sql))
                    .await
                    .map(|result| truncate_result_with_row_limit(result, row_limit))
            }
        }
        PoolKind::Elasticsearch(client) => {
            let client = client.clone();
            let sql = sql.to_string();
            drop(connections);
            wait_for_query(cancel_token, db::elasticsearch_driver::execute_rest_query(&client, &sql))
                .await
                .map(|result| truncate_result_with_row_limit(result, row_limit))
        }
        PoolKind::Redis(_) => Err("Use Redis-specific commands".to_string()),
        PoolKind::MongoDb(_) => Err("Use MongoDB-specific commands".to_string()),
        PoolKind::Dameng(client) => {
            let client = client.clone();
            let sql = sql.to_string();
            let schema = schema.map(|s| s.to_string());
            drop(connections);
            wait_for_query(cancel_token, async move {
                let task = tokio::task::spawn_blocking(move || {
                    let client = client.lock().map_err(|e| e.to_string())?;
                    if let Some(schema) = schema {
                        db::dm_driver::execute_query_with_schema_sync(&client, &schema, &sql)
                    } else {
                        db::dm_driver::execute_query_sync(&client, &sql)
                    }
                });
                task.await.map_err(|e| e.to_string())?
            })
            .await
            .map(|result| truncate_result_with_row_limit(result, row_limit))
        }
        PoolKind::Gaussdb(client) => {
            let client = client.clone();
            let sql = sql.to_string();
            drop(connections);
            wait_for_query(cancel_token, async move {
                let mut client = client.lock().await;
                db::gaussdb_driver::execute_query(&mut client, &sql).await
            })
            .await
            .map(|result| truncate_result_with_row_limit(result, row_limit))
        }
        PoolKind::ExternalTabular(ext_pool) => {
            if !starts_with_executable_sql_keyword(sql, &["SELECT", "WITH", "SHOW", "DESCRIBE", "EXPLAIN", "PRAGMA"]) {
                return Err("External data sources are read-only. Only SELECT queries are supported.".to_string());
            }
            let con = ext_pool.cache.clone();
            let sql = sql.to_string();
            drop(connections);
            wait_for_query(cancel_token, async move {
                let task = tokio::task::spawn_blocking(move || {
                    let con = con.lock().map_err(|e| e.to_string())?;
                    duckdb_execute_with_row_limit(&con, &sql, row_limit)
                });
                task.await.map_err(|e| e.to_string())?
            })
            .await
        }
        PoolKind::ExternalDriver { config, session, .. } => {
            let config = config.clone();
            let session = session.clone();
            let sql = sql.to_string();
            let schema = schema.map(str::to_string);
            drop(connections);
            wait_for_query(cancel_token, async move {
                let params = serde_json::json!({
                    "connection": config,
                    "sql": sql,
                    "schema": schema,
                });
                session.invoke::<db::QueryResult>("executeQuery", params).await
            })
            .await
            .map(|result| truncate_result_with_row_limit(result, row_limit))
        }
    }
}

pub async fn execute_sql_statement(
    state: &AppState,
    connection_id: &str,
    database: &str,
    sql: &str,
    schema: Option<&str>,
    cancel_token: Option<CancellationToken>,
) -> Result<db::QueryResult, String> {
    execute_sql_statement_with_row_limit(state, connection_id, database, sql, schema, cancel_token, MAX_ROWS).await
}

pub async fn execute_sql_statement_with_row_limit(
    state: &AppState,
    connection_id: &str,
    database: &str,
    sql: &str,
    schema: Option<&str>,
    cancel_token: Option<CancellationToken>,
    row_limit: usize,
) -> Result<db::QueryResult, String> {
    let row_limit = row_limit.max(1);
    let pool_key = if database.is_empty() {
        connection_id.to_string()
    } else {
        state.get_or_create_pool(connection_id, Some(database)).await?
    };

    if is_canceled(&cancel_token) {
        return Err(canceled_error());
    }

    let result = do_execute_with_row_limit(state, &pool_key, sql, schema, cancel_token.clone(), row_limit).await;

    let result = match &result {
        Err(e) if is_connection_error(e) && !is_canceled(&cancel_token) => {
            let db_opt = if database.is_empty() { None } else { Some(database) };
            let new_key = state.reconnect_pool(connection_id, db_opt).await?;
            do_execute_with_row_limit(state, &new_key, sql, schema, cancel_token, row_limit).await
        }
        _ => result,
    };

    result.map(|result| truncate_result_with_row_limit(result, row_limit))
}

pub async fn execute_multi_core(
    state: &AppState,
    connection_id: &str,
    database: &str,
    sql: &str,
    schema: Option<&str>,
    cancel_token: Option<CancellationToken>,
) -> Result<Vec<db::QueryResult>, String> {
    let pool_key = if database.is_empty() {
        connection_id.to_string()
    } else {
        state.get_or_create_pool(connection_id, Some(database)).await?
    };

    let is_sqlserver = {
        let connections = state.connections.read().await;
        matches!(connections.get(&pool_key), Some(PoolKind::SqlServer(_)))
    };

    if is_sqlserver {
        return execute_multi_sqlserver(state, &pool_key, sql, cancel_token).await;
    }

    let statements = split_sql_statements(sql);
    if statements.len() <= 1 {
        let single_sql = statements.into_iter().next().unwrap_or_default();
        let result = execute_sql_statement(state, connection_id, database, &single_sql, schema, cancel_token).await?;
        return Ok(vec![result]);
    }

    let mut results = Vec::with_capacity(statements.len());
    for stmt in &statements {
        if is_canceled(&cancel_token) {
            results.push(db::QueryResult {
                columns: vec!["Error".to_string()],
                rows: vec![vec![serde_json::Value::String(canceled_error())]],
                affected_rows: 0,
                execution_time_ms: 0,
                truncated: false,
            });
            break;
        }
        match execute_sql_statement(state, connection_id, database, stmt, schema, cancel_token.clone()).await {
            Ok(r) => results.push(r),
            Err(e) => {
                results.push(db::QueryResult {
                    columns: vec!["Error".to_string()],
                    rows: vec![vec![serde_json::Value::String(e)]],
                    affected_rows: 0,
                    execution_time_ms: 0,
                    truncated: false,
                });
            }
        }
    }

    Ok(results)
}

async fn execute_multi_sqlserver(
    state: &AppState,
    pool_key: &str,
    sql: &str,
    cancel_token: Option<CancellationToken>,
) -> Result<Vec<db::QueryResult>, String> {
    let batches = split_sql_batches(sql);
    let mut all_results = Vec::new();

    for batch in &batches {
        if is_canceled(&cancel_token) {
            all_results.push(db::QueryResult {
                columns: vec!["Error".to_string()],
                rows: vec![vec![serde_json::Value::String(canceled_error())]],
                affected_rows: 0,
                execution_time_ms: 0,
                truncated: false,
            });
            break;
        }

        let connections = state.connections.read().await;
        let pool = connections.get(pool_key).ok_or("Connection not found")?;
        let client = match pool {
            PoolKind::SqlServer(c) => c.clone(),
            _ => return Err("Expected SQL Server connection".to_string()),
        };
        drop(connections);

        let mut client = match cancel_token.as_ref() {
            Some(token) => tokio::select! {
                biased;
                _ = token.cancelled() => return Err(canceled_error()),
                guard = client.lock() => guard,
            },
            None => client.lock().await,
        };

        match db::sqlserver::execute_batch(&mut client, batch).await {
            Ok(results) => all_results.extend(results),
            Err(e) => {
                all_results.push(db::QueryResult {
                    columns: vec!["Error".to_string()],
                    rows: vec![vec![serde_json::Value::String(e)]],
                    affected_rows: 0,
                    execution_time_ms: 0,
                    truncated: false,
                });
            }
        }
    }

    if all_results.is_empty() {
        all_results.push(db::QueryResult {
            columns: vec![],
            rows: vec![],
            affected_rows: 0,
            execution_time_ms: 0,
            truncated: false,
        });
    }

    Ok(all_results)
}

pub async fn execute_statements(
    state: &AppState,
    connection_id: &str,
    database: &str,
    statements: &[String],
    schema: Option<&str>,
) -> Result<db::QueryResult, String> {
    let pool_key = if database.is_empty() {
        connection_id.to_string()
    } else {
        state.get_or_create_pool(connection_id, Some(database)).await?
    };

    let mut total_affected: u64 = 0;
    let start = std::time::Instant::now();

    for (i, sql) in statements.iter().enumerate() {
        match do_execute(state, &pool_key, sql, schema, None).await {
            Ok(result) => {
                total_affected += result.affected_rows;
            }
            Err(e) => {
                if is_connection_error(&e) {
                    let db_opt = if database.is_empty() { None } else { Some(database) };
                    let _ = state.reconnect_pool(connection_id, db_opt).await;
                }
                return Err(format!(
                    "Statement {} failed: {}. Previous {} statement(s) may have been committed.",
                    i + 1,
                    e,
                    i
                ));
            }
        }
    }

    Ok(db::QueryResult {
        columns: vec![],
        rows: vec![],
        affected_rows: total_affected,
        execution_time_ms: start.elapsed().as_millis(),
        truncated: false,
    })
}

/// Execute multiple SQL statements within a single transaction.
/// For sqlx-based pools (Postgres/MySQL/SQLite), uses the Transaction API to
/// guarantee all statements run on the same physical connection.
/// For custom drivers (ClickHouse/SqlServer/Dameng/Gaussdb), uses explicit
/// BEGIN/COMMIT/ROLLBACK on the already-single-connection client.
/// For databases that don't support explicit transactions (Redis, MongoDB, Oracle),
/// executes statements sequentially without transaction.
/// If BEGIN fails, returns an error — no silent fallback to auto-commit.
pub async fn execute_statements_in_transaction(
    state: &AppState,
    connection_id: &str,
    database: &str,
    statements: &[String],
    schema: Option<&str>,
) -> Result<db::QueryResult, String> {
    let pool_key = if database.is_empty() {
        connection_id.to_string()
    } else {
        state.get_or_create_pool(connection_id, Some(database)).await?
    };

    let start = std::time::Instant::now();

    // Clone the pool handle within the lock, then drop it before any async work.
    let path = {
        let conns = state.connections.read().await;
        conns.get(&pool_key).map(|p| match p {
            PoolKind::Postgres(pg) => TxPath::Pg(pg.clone()),
            PoolKind::Mysql(mp, _mode) => TxPath::Mysql(mp.clone(), false),
            PoolKind::Sqlite(sq) => TxPath::Sqlite(sq.clone()),
            PoolKind::ClickHouse(_) | PoolKind::SqlServer(_) | PoolKind::Dameng(_) | PoolKind::Gaussdb(_) => {
                TxPath::Explicit
            }
            PoolKind::DuckDb(_)
            | PoolKind::Redis(_)
            | PoolKind::MongoDb(_)
            | PoolKind::Oracle(_)
            | PoolKind::Elasticsearch(_)
            | PoolKind::ExternalTabular(_)
            | PoolKind::ExternalDriver { .. } => TxPath::None,
        })
    };

    match path {
        Some(TxPath::Pg(pool)) => exec_tx_pg_inner(pool, statements, schema, start).await,
        Some(TxPath::Mysql(pool, _bare)) => exec_tx_mysql_inner(pool, statements, start).await,
        Some(TxPath::Sqlite(pool)) => exec_tx_sqlite_inner(pool, statements, start).await,
        Some(TxPath::Explicit) => exec_tx_explicit_inner(state, &pool_key, statements, schema, start).await,
        Some(TxPath::None) => exec_tx_none_inner(state, &pool_key, statements, schema, start).await,
        None => Err("Connection not found for transaction".to_string()),
    }
}

/// Owned pool variants for safe dispatch across async boundaries.
enum TxPath {
    Pg(sqlx::postgres::PgPool),
    Mysql(sqlx::mysql::MySqlPool, bool),
    Sqlite(sqlx::sqlite::SqlitePool),
    Explicit,
    None,
}

// Each of these acquires a dedicated connection and runs all statements within
// BEGIN ... COMMIT/ROLLBACK, guaranteeing a single physical connection.
// This avoids sqlx::Transaction<T> which has Send/lifetime incompatibility with Tauri macro.

async fn exec_tx_pg_inner(
    pool: sqlx::postgres::PgPool,
    statements: &[String],
    schema: Option<&str>,
    start: std::time::Instant,
) -> Result<db::QueryResult, String> {
    let mut conn = pool.acquire().await.map_err(|e| format!("Failed to acquire connection: {}", e))?;
    // Set schema first
    if let Some(s) = schema {
        let sp = format!("SET search_path TO \"{}\", public", s);
        sqlx::query(&sp).execute(&mut *conn).await.map_err(|e| format!("SET search_path failed: {}", e))?;
    }
    sqlx::query("BEGIN").execute(&mut *conn).await.map_err(|e| format!("Failed to begin transaction: {}", e))?;
    let mut total_affected: u64 = 0;
    for (i, sql) in statements.iter().enumerate() {
        match sqlx::query(sql).execute(&mut *conn).await {
            Ok(r) => total_affected += r.rows_affected(),
            Err(e) => {
                let _ = sqlx::query("ROLLBACK").execute(&mut *conn).await;
                return Err(format!("Statement {} failed: {}", i + 1, e));
            }
        }
    }
    sqlx::query("COMMIT").execute(&mut *conn).await.map_err(|e| format!("COMMIT failed: {}", e))?;
    Ok(db::QueryResult {
        columns: vec![],
        rows: vec![],
        affected_rows: total_affected,
        execution_time_ms: start.elapsed().as_millis(),
        truncated: false,
    })
}

async fn exec_tx_mysql_inner(
    pool: sqlx::mysql::MySqlPool,
    statements: &[String],
    start: std::time::Instant,
) -> Result<db::QueryResult, String> {
    let statements = statements.to_vec();
    tokio::task::spawn_blocking(move || {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| format!("Failed to start MySQL transaction runtime: {}", e))?
            .block_on(exec_tx_mysql_raw_inner(pool, statements, start))
    })
    .await
    .map_err(|e| format!("MySQL transaction task failed: {}", e))?
}

async fn exec_tx_mysql_raw_inner(
    pool: sqlx::mysql::MySqlPool,
    statements: Vec<String>,
    start: std::time::Instant,
) -> Result<db::QueryResult, String> {
    let mut conn = pool.acquire().await.map_err(|e| format!("Failed to acquire connection: {}", e))?;
    sqlx::raw_sql("START TRANSACTION")
        .execute(&mut *conn)
        .await
        .map_err(|e| format!("Failed to begin transaction: {}", e))?;
    let mut total_affected: u64 = 0;
    for (i, sql) in statements.iter().enumerate() {
        match sqlx::raw_sql(sql).execute(&mut *conn).await {
            Ok(r) => total_affected += r.rows_affected(),
            Err(e) => {
                let _ = sqlx::raw_sql("ROLLBACK").execute(&mut *conn).await;
                return Err(format!("Statement {} failed: {}", i + 1, e));
            }
        }
    }
    sqlx::raw_sql("COMMIT").execute(&mut *conn).await.map_err(|e| format!("COMMIT failed: {}", e))?;
    Ok(db::QueryResult {
        columns: vec![],
        rows: vec![],
        affected_rows: total_affected,
        execution_time_ms: start.elapsed().as_millis(),
        truncated: false,
    })
}

async fn exec_tx_sqlite_inner(
    pool: sqlx::sqlite::SqlitePool,
    statements: &[String],
    start: std::time::Instant,
) -> Result<db::QueryResult, String> {
    let mut conn = pool.acquire().await.map_err(|e| format!("Failed to acquire connection: {}", e))?;
    sqlx::query("BEGIN").execute(&mut *conn).await.map_err(|e| format!("Failed to begin transaction: {}", e))?;
    let mut total_affected: u64 = 0;
    for (i, sql) in statements.iter().enumerate() {
        match sqlx::query(sql).execute(&mut *conn).await {
            Ok(r) => total_affected += r.rows_affected(),
            Err(e) => {
                let _ = sqlx::query("ROLLBACK").execute(&mut *conn).await;
                return Err(format!("Statement {} failed: {}", i + 1, e));
            }
        }
    }
    sqlx::query("COMMIT").execute(&mut *conn).await.map_err(|e| format!("COMMIT failed: {}", e))?;
    Ok(db::QueryResult {
        columns: vec![],
        rows: vec![],
        affected_rows: total_affected,
        execution_time_ms: start.elapsed().as_millis(),
        truncated: false,
    })
}

async fn exec_tx_explicit_inner(
    state: &AppState,
    pool_key: &str,
    statements: &[String],
    schema: Option<&str>,
    start: std::time::Instant,
) -> Result<db::QueryResult, String> {
    do_execute(state, pool_key, "BEGIN", schema, None)
        .await
        .map_err(|e| format!("Failed to begin transaction: {}", e))?;

    let mut total_affected: u64 = 0;
    for (i, sql) in statements.iter().enumerate() {
        match do_execute(state, pool_key, sql, schema, None).await {
            Ok(result) => {
                total_affected += result.affected_rows;
            }
            Err(e) => {
                if let Err(rb_err) = do_execute(state, pool_key, "ROLLBACK", schema, None).await {
                    log::error!("ROLLBACK failed after statement {} error: {}", i + 1, rb_err);
                }
                return Err(format!("Statement {} failed: {}", i + 1, e));
            }
        }
    }

    do_execute(state, pool_key, "COMMIT", schema, None).await.map_err(|e| format!("COMMIT failed: {}", e))?;

    Ok(db::QueryResult {
        columns: vec![],
        rows: vec![],
        affected_rows: total_affected,
        execution_time_ms: start.elapsed().as_millis(),
        truncated: false,
    })
}

async fn exec_tx_none_inner(
    state: &AppState,
    pool_key: &str,
    statements: &[String],
    schema: Option<&str>,
    start: std::time::Instant,
) -> Result<db::QueryResult, String> {
    let mut total_affected: u64 = 0;
    for (i, sql) in statements.iter().enumerate() {
        log::info!("[query][tx-none:statement:start] index={} sql={}", i + 1, sql);
        match do_execute(state, pool_key, sql, schema, None).await {
            Ok(result) => {
                total_affected += result.affected_rows;
                log::info!("[query][tx-none:statement:done] index={} affected_rows={}", i + 1, result.affected_rows);
            }
            Err(e) => {
                log::warn!("Statement {} failed (no transaction support): {}", i + 1, e);
                return Err(format!(
                    "Statement {} failed: {}. No transaction support for this database type.",
                    i + 1,
                    e
                ));
            }
        }
    }

    Ok(db::QueryResult {
        columns: vec![],
        rows: vec![],
        affected_rows: total_affected,
        execution_time_ms: start.elapsed().as_millis(),
        truncated: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::connection::AppState;
    use crate::models::connection::{ConnectionConfig, DatabaseType};
    use crate::storage::Storage;

    fn query_result_with_rows(row_count: usize, truncated: bool) -> db::QueryResult {
        db::QueryResult {
            columns: vec!["id".to_string()],
            rows: (1..=row_count).map(|id| vec![serde_json::Value::Number(id.into())]).collect(),
            affected_rows: 0,
            execution_time_ms: 0,
            truncated,
        }
    }

    #[test]
    fn truncate_result_with_row_limit_truncates_rows_and_marks_result() {
        let result = truncate_result_with_row_limit(query_result_with_rows(5, false), 3);

        assert_eq!(result.rows.len(), 3);
        assert!(result.truncated);
        assert_eq!(result.rows[0][0], serde_json::Value::Number(1.into()));
        assert_eq!(result.rows[2][0], serde_json::Value::Number(3.into()));
    }

    #[test]
    fn truncate_result_with_row_limit_keeps_untruncated_result_when_within_limit() {
        let result = truncate_result_with_row_limit(query_result_with_rows(3, false), 3);

        assert_eq!(result.rows.len(), 3);
        assert!(!result.truncated);
    }

    #[test]
    fn truncate_result_with_row_limit_treats_zero_limit_as_one() {
        let result = truncate_result_with_row_limit(query_result_with_rows(2, false), 0);

        assert_eq!(result.rows.len(), 1);
        assert!(result.truncated);
    }

    #[test]
    fn mysql_and_postgres_pool_branches_call_limit_aware_drivers() {
        let source = include_str!("query.rs");

        assert!(source.contains("db::mysql::execute_query_with_row_limit(&p, sql, bare, row_limit)"));
        assert!(source.contains("db::postgres::execute_query_with_schema_and_row_limit(&p, &schema, sql, row_limit)"));
        assert!(source.contains("db::postgres::execute_query_with_row_limit(&p, sql, row_limit)"));
    }

    #[tokio::test]
    async fn wait_for_query_returns_cancelled_when_token_is_cancelled() {
        let token = CancellationToken::new();
        token.cancel();

        let result = wait_for_query(Some(token), async {
            tokio::time::sleep(Duration::from_secs(30)).await;
            Ok(db::QueryResult {
                columns: vec![],
                rows: vec![],
                affected_rows: 0,
                execution_time_ms: 0,
                truncated: false,
            })
        })
        .await;

        assert_eq!(result.unwrap_err(), QUERY_CANCELED);
    }

    #[tokio::test]
    async fn wait_for_query_without_token_still_times_out() {
        let result = wait_for_query_with_timeout(None, Duration::from_millis(10), async {
            tokio::time::sleep(Duration::from_secs(1)).await;
            Ok(db::QueryResult {
                columns: vec![],
                rows: vec![],
                affected_rows: 0,
                execution_time_ms: 0,
                truncated: false,
            })
        })
        .await;

        assert_eq!(result.unwrap_err(), timeout_error());
    }

    #[test]
    fn is_connection_error_detects_english_messages() {
        assert!(is_connection_error("connection reset"));
        assert!(is_connection_error("broken pipe"));
        assert!(is_connection_error("reset by peer"));
        assert!(is_connection_error("Connection timed out"));
        assert!(is_connection_error("socket closed"));
        assert!(is_connection_error("unexpected eof"));
    }

    #[test]
    fn is_connection_error_detects_localized_io_errors() {
        assert!(is_connection_error("I/O error: 远程主机强迫关闭了一个现有的连接。 (os error 10054)"));
        assert!(is_connection_error(
            "I/O error: 由于连接方在一段时间后没有正确答复或连接的主机没有反应，连接尝试失败。 (os error 10060)"
        ));
    }

    #[test]
    fn is_connection_error_detects_os_error_codes() {
        assert!(is_connection_error("os error 10053"));
        assert!(is_connection_error("os error 10054"));
        assert!(is_connection_error("os error 10060"));
        assert!(is_connection_error("os error 10061"));
    }

    #[test]
    fn is_connection_error_rejects_non_connection_errors() {
        assert!(!is_connection_error("ORA-00942: table or view does not exist"));
        assert!(!is_connection_error("syntax error at position 5"));
        assert!(!is_connection_error("os error 13"));
    }

    fn sqlite_config(path: &std::path::Path) -> ConnectionConfig {
        ConnectionConfig {
            id: "sqlite-id".to_string(),
            name: "local-sqlite".to_string(),
            db_type: DatabaseType::Sqlite,
            driver_profile: None,
            driver_label: None,
            url_params: None,
            host: path.display().to_string(),
            port: 0,
            username: String::new(),
            password: String::new(),
            database: None,
            color: None,
            ssh_enabled: false,
            ssh_host: String::new(),
            ssh_port: 22,
            ssh_user: String::new(),
            ssh_password: String::new(),
            ssh_key_path: String::new(),
            ssh_key_passphrase: String::new(),
            ssh_expose_lan: false,
            ssh_connect_timeout_secs: crate::models::connection::default_ssh_connect_timeout_secs(),
            proxy_enabled: false,
            proxy_type: crate::models::connection::ProxyType::Socks5,
            proxy_host: String::new(),
            proxy_port: 1080,
            proxy_username: String::new(),
            proxy_password: String::new(),
            ssl: false,
            sysdba: false,
            connection_string: None,
            external_config: None,
            jdbc_driver_class: None,
            jdbc_driver_paths: Vec::new(),
        }
    }

    async fn sqlite_state_with_rows(row_count: usize) -> (AppState, std::path::PathBuf, std::path::PathBuf) {
        let unique = uuid::Uuid::new_v4();
        let data_path = std::env::temp_dir().join(format!("dbx-query-data-{unique}.sqlite"));
        let storage_path = std::env::temp_dir().join(format!("dbx-query-storage-{unique}.sqlite"));
        std::fs::File::create(&data_path).unwrap();
        let pool = db::sqlite::connect_path(&data_path.display().to_string()).await.unwrap();
        db::sqlite::execute_query(&pool, "CREATE TABLE numbers (id INTEGER PRIMARY KEY)").await.unwrap();
        for id in 1..=row_count {
            sqlx::query("INSERT INTO numbers (id) VALUES (?)").bind(id as i64).execute(&pool).await.unwrap();
        }
        pool.close().await;

        let storage = Storage::open(&storage_path).await.unwrap();
        let state = AppState::new(storage);
        let config = sqlite_config(&data_path);
        state.configs.write().await.insert(config.id.clone(), config);
        let pool = db::sqlite::connect_path(&data_path.display().to_string()).await.unwrap();
        state.connections.write().await.insert("sqlite-id".to_string(), PoolKind::Sqlite(pool));
        (state, data_path, storage_path)
    }

    #[tokio::test]
    async fn execute_sql_statement_with_row_limit_caps_sqlite_rows() {
        let (state, data_path, storage_path) = sqlite_state_with_rows(200).await;

        let result = execute_sql_statement_with_row_limit(
            &state,
            "sqlite-id",
            "",
            "SELECT id FROM numbers ORDER BY id",
            None,
            None,
            7,
        )
        .await
        .unwrap();

        assert!(result.rows.len() <= 7);
        assert_eq!(result.rows.len(), 7);
        assert!(result.truncated);

        let _ = std::fs::remove_file(data_path);
        let _ = std::fs::remove_file(storage_path);
    }
}
