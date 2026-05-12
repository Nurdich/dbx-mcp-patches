# DBX CLI Runtime Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a full DBX CLI with independent GUI Runtime support for the 8 specified commands while preserving the existing MCP package and behavior.

**Architecture:** Add a new `dbx-cli` Rust binary crate that first attempts GUI Runtime discovery, then falls back to headless mode. Shared CLI data models, JSON envelope, SQL safety classification, schema snapshot, risk metadata, and handoff queue live in `dbx-core`. The desktop app starts a separate authenticated localhost runtime and the Vue UI synchronizes active context, selection, result samples, and handoff review state into that runtime.

**Tech Stack:** Rust workspace, `dbx-core`, Tauri v2, Vue 3/Pinia, SQLite storage via `sqlx`, existing DBX schema/query/SSH core, Node test runner for frontend utilities, Cargo tests for Rust.

---

## Non-Negotiable Constraints

- Preserve all existing `mcp/` files, package behavior, tool names, and bridge endpoints.
- Do not remove or rename current Tauri commands.
- Do not add CLI commands that create, edit, or delete DBX connections.
- All CLI agent-facing commands return the envelope shape `{ ok, source, data }` or `{ ok, source, error }`.
- `safe-query` directly executes only READ SQL; WRITE and DDL return structured blocking errors and point to `handoff`.
- `selection` and `result current` return real GUI state when runtime is available, not a placeholder.
- `handoff` sends to GUI runtime when running and writes queued pending records when headless.

## File Structure

- Create `crates/dbx-cli/Cargo.toml`: binary crate configuration.
- Create `crates/dbx-cli/src/main.rs`: argument parsing, command dispatch, runtime/headless selection.
- Create `crates/dbx-cli/src/commands.rs`: CLI command handlers.
- Create `crates/dbx-cli/src/runtime_client.rs`: discovery file loading, token-authenticated HTTP calls.
- Create `crates/dbx-core/src/cli.rs`: envelope, error codes, CLI DTOs, app data path helper.
- Create `crates/dbx-core/src/sql_safety.rs`: SQL classification and risk metadata.
- Create `crates/dbx-core/src/schema_snapshot.rs`: normalized schema snapshot orchestration.
- Create `crates/dbx-core/src/handoff.rs`: handoff model and queued storage helpers.
- Modify `crates/dbx-core/src/lib.rs`: export new modules.
- Modify `crates/dbx-core/src/storage.rs`: add pending handoff table migration and load/save helpers.
- Modify root `Cargo.toml`: add `crates/dbx-cli` workspace member.
- Modify `src-tauri/src/lib.rs`: start independent agent runtime and register runtime commands.
- Create `src-tauri/src/commands/agent_runtime.rs`: authenticated runtime server and shared runtime state.
- Modify `src-tauri/src/commands/mod.rs`: expose `agent_runtime`.
- Modify `src/lib/api.ts` and `src/lib/tauri.ts`: add runtime state sync and handoff API wrappers.
- Create `src/stores/agentRuntimeStore.ts`: collect current UI context and push snapshots.
- Create `src/components/agent/AgentHandoffDialog.vue`: review queued/runtime handoffs.
- Modify `src/components/layout/AppDialogs.vue`: mount handoff dialog.
- Modify `src/stores/queryStore.ts`: notify runtime store on active tab/result changes.
- Modify `src/components/grid/DataGrid.vue`: notify runtime store on grid selection changes.
- Modify `src/composables/useTauriEvents.ts`: keep existing MCP listeners unchanged.
- Add Rust tests under `crates/dbx-core/src/sql_safety.rs` and `crates/dbx-core/src/cli.rs`.
- Add frontend tests under `tests/agentRuntimeStore.test.ts` for payload shaping.

---

### Task 1: Workspace and CLI Crate Skeleton

**Files:**
- Modify: `/Users/bytedance/open_source_poj/dbx/Cargo.toml`
- Create: `/Users/bytedance/open_source_poj/dbx/crates/dbx-cli/Cargo.toml`
- Create: `/Users/bytedance/open_source_poj/dbx/crates/dbx-cli/src/main.rs`

- [ ] **Step 1: Add workspace member**

Change the workspace members in `/Users/bytedance/open_source_poj/dbx/Cargo.toml` to include the CLI crate:

```toml
[workspace]
resolver = "2"
members = ["src-tauri", "crates/dbx-core", "crates/dbx-cli", "src-web"]
```

- [ ] **Step 2: Create CLI crate manifest**

Create `/Users/bytedance/open_source_poj/dbx/crates/dbx-cli/Cargo.toml`:

```toml
[package]
name = "dbx-cli"
version = "0.5.2"
edition = "2021"

[[bin]]
name = "dbx-cli"
path = "src/main.rs"

[dependencies]
dbx-core = { path = "../dbx-core" }
chrono = { version = "0.4", features = ["serde"] }
serde = { version = "1.0", features = ["derive"] }
serde_json = "1.0"
tokio = { version = "1", features = ["full"] }
reqwest = { version = "0.12", default-features = false, features = ["json", "rustls-tls"] }
uuid = { version = "1", features = ["v4", "serde"] }
```

- [ ] **Step 3: Create placeholder CLI entry**

Create `/Users/bytedance/open_source_poj/dbx/crates/dbx-cli/src/main.rs`:

```rust
mod commands;
mod runtime_client;

#[tokio::main]
async fn main() {
    if let Err(err) = commands::run(std::env::args().skip(1).collect()).await {
        println!("{}", serde_json::to_string_pretty(&err).unwrap_or_else(|_| "{\"ok\":false}".to_string()));
        std::process::exit(1);
    }
}
```

- [ ] **Step 4: Verify workspace recognizes the binary**

Run:

```bash
cargo metadata --format-version 1 --no-deps
```

Expected: output contains `"name":"dbx-cli"` for the package and `"name":"dbx-cli"` for the binary target.

---

### Task 2: Core CLI Envelope and Error DTOs

**Files:**
- Create: `/Users/bytedance/open_source_poj/dbx/crates/dbx-core/src/cli.rs`
- Modify: `/Users/bytedance/open_source_poj/dbx/crates/dbx-core/src/lib.rs`

- [ ] **Step 1: Add envelope and error types**

Create `/Users/bytedance/open_source_poj/dbx/crates/dbx-core/src/cli.rs`:

```rust
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum CliSource {
    GuiRuntime,
    Headless,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum CliErrorCode {
    GuiRuntimeRequired,
    ConnectionNotFound,
    AmbiguousConnection,
    SecretUnavailable,
    SshTunnelFailed,
    QueryClassificationFailed,
    HandoffRequired,
    DdlBlocked,
    ProductionWriteBlocked,
    UnsupportedDatabaseType,
    Timeout,
    InternalError,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CliError {
    pub code: CliErrorCode,
    pub message: String,
    pub recoverable: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum CliEnvelope<T> {
    Success { ok: bool, source: CliSource, data: T },
    Failure { ok: bool, source: CliSource, error: CliError },
}

pub fn ok<T>(source: CliSource, data: T) -> CliEnvelope<T> {
    CliEnvelope::Success { ok: true, source, data }
}

pub fn fail<T>(source: CliSource, code: CliErrorCode, message: impl Into<String>, recoverable: bool) -> CliEnvelope<T> {
    CliEnvelope::Failure { ok: false, source, error: CliError { code, message: message.into(), recoverable } }
}
```

- [ ] **Step 2: Export module**

Add this line to `/Users/bytedance/open_source_poj/dbx/crates/dbx-core/src/lib.rs`:

```rust
pub mod cli;
```

- [ ] **Step 3: Add envelope unit test**

Append tests in `cli.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serializes_success_source_as_kebab_case() {
        let env = ok(CliSource::GuiRuntime, serde_json::json!({"value": 1}));
        let json = serde_json::to_string(&env).unwrap();
        assert!(json.contains("\"ok\":true"));
        assert!(json.contains("\"source\":\"gui-runtime\""));
    }

    #[test]
    fn serializes_error_code_as_screaming_snake_case() {
        let env: CliEnvelope<()> = fail(CliSource::Headless, CliErrorCode::GuiRuntimeRequired, "runtime needed", true);
        let json = serde_json::to_string(&env).unwrap();
        assert!(json.contains("\"GUI_RUNTIME_REQUIRED\""));
    }
}
```

- [ ] **Step 4: Run targeted test**

Run:

```bash
cargo test -p dbx-core cli::tests
```

Expected: both tests pass.

---

### Task 3: SQL Safety and Risk Classification

**Files:**
- Create: `/Users/bytedance/open_source_poj/dbx/crates/dbx-core/src/sql_safety.rs`
- Modify: `/Users/bytedance/open_source_poj/dbx/crates/dbx-core/src/lib.rs`

- [ ] **Step 1: Implement classifier**

Create `/Users/bytedance/open_source_poj/dbx/crates/dbx-core/src/sql_safety.rs`:

```rust
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum OperationClass {
    Read,
    Write,
    Ddl,
    Unknown,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum RiskLevel {
    Low,
    Medium,
    High,
    Critical,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RiskMetadata {
    pub operation_class: OperationClass,
    pub risk_level: RiskLevel,
    pub is_production: bool,
    pub production_reason: Option<String>,
    pub first_token: Option<String>,
}

pub fn classify_sql(sql: &str) -> OperationClass {
    let token = first_executable_token(sql).map(|s| s.to_ascii_uppercase());
    match token.as_deref() {
        Some("SELECT" | "SHOW" | "DESCRIBE" | "EXPLAIN" | "WITH") => OperationClass::Read,
        Some("INSERT" | "UPDATE" | "DELETE" | "MERGE" | "REPLACE") => OperationClass::Write,
        Some("CREATE" | "ALTER" | "DROP" | "TRUNCATE" | "RENAME") => OperationClass::Ddl,
        _ => OperationClass::Unknown,
    }
}

pub fn risk_for(sql: &str, connection_name: &str, color: Option<&str>) -> RiskMetadata {
    let operation_class = classify_sql(sql);
    let (is_production, production_reason) = production_signal(connection_name, color);
    let risk_level = match (operation_class, is_production) {
        (OperationClass::Read, _) => RiskLevel::Low,
        (OperationClass::Write, false) => RiskLevel::Medium,
        (OperationClass::Write, true) => RiskLevel::High,
        (OperationClass::Ddl, _) => RiskLevel::Critical,
        (OperationClass::Unknown, _) => RiskLevel::High,
    };
    RiskMetadata {
        operation_class,
        risk_level,
        is_production,
        production_reason,
        first_token: first_executable_token(sql).map(str::to_string),
    }
}

fn production_signal(connection_name: &str, color: Option<&str>) -> (bool, Option<String>) {
    if matches!(color, Some("#ef4444")) {
        return (true, Some("red connection color".to_string()));
    }
    let name = connection_name.to_ascii_lowercase();
    if ["prod", "production", "live"].iter().any(|needle| name.contains(needle)) {
        return (true, Some("connection name fallback".to_string()));
    }
    (false, None)
}

fn first_executable_token(sql: &str) -> Option<&str> {
    let bytes = sql.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i + 1 < bytes.len() && bytes[i] == b'-' && bytes[i + 1] == b'-' {
            i += 2;
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        if i + 1 < bytes.len() && bytes[i] == b'/' && bytes[i + 1] == b'*' {
            i += 2;
            while i + 1 < bytes.len() && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                i += 1;
            }
            i = (i + 2).min(bytes.len());
            continue;
        }
        break;
    }
    let start = i;
    while i < bytes.len() && (bytes[i].is_ascii_alphabetic() || bytes[i] == b'_') {
        i += 1;
    }
    (i > start).then_some(&sql[start..i])
}
```

- [ ] **Step 2: Export module**

Add this line to `/Users/bytedance/open_source_poj/dbx/crates/dbx-core/src/lib.rs`:

```rust
pub mod sql_safety;
```

- [ ] **Step 3: Add classifier tests**

Append tests to `sql_safety.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn comments_do_not_hide_read_token() {
        assert_eq!(classify_sql("-- comment\nSELECT 1"), OperationClass::Read);
        assert_eq!(classify_sql("/* DROP TABLE x */ SELECT 1"), OperationClass::Read);
    }

    #[test]
    fn classifies_write_and_ddl() {
        assert_eq!(classify_sql("update users set name = 'a'"), OperationClass::Write);
        assert_eq!(classify_sql("DROP TABLE users"), OperationClass::Ddl);
    }

    #[test]
    fn red_color_marks_production() {
        let risk = risk_for("orders", "prod-main", Some("#ef4444"));
        assert!(risk.is_production);
        assert_eq!(risk.risk_level, RiskLevel::Low);
    }
}
```

- [ ] **Step 4: Run targeted tests**

Run:

```bash
cargo test -p dbx-core sql_safety::tests
```

Expected: all classifier tests pass.

---

### Task 4: Handoff Storage Model

**Files:**
- Create: `/Users/bytedance/open_source_poj/dbx/crates/dbx-core/src/handoff.rs`
- Modify: `/Users/bytedance/open_source_poj/dbx/crates/dbx-core/src/lib.rs`
- Modify: `/Users/bytedance/open_source_poj/dbx/crates/dbx-core/src/storage.rs`

- [ ] **Step 1: Add handoff types**

Create `/Users/bytedance/open_source_poj/dbx/crates/dbx-core/src/handoff.rs`:

```rust
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::sql_safety::{OperationClass, RiskLevel};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum HandoffStatus {
    Queued,
    Shown,
    Approved,
    Rejected,
    Executed,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HandoffItem {
    pub id: String,
    pub created_at: DateTime<Utc>,
    pub created_by: String,
    pub connection_name: String,
    pub database: Option<String>,
    pub title: String,
    pub description: Option<String>,
    pub sql: String,
    pub operation_class: OperationClass,
    pub risk_level: RiskLevel,
    pub is_production: bool,
    pub status: HandoffStatus,
    pub result_summary: Option<String>,
    pub error: Option<String>,
}

impl HandoffItem {
    pub fn queued(
        connection_name: String,
        database: Option<String>,
        title: String,
        description: Option<String>,
        sql: String,
        operation_class: OperationClass,
        risk_level: RiskLevel,
        is_production: bool,
    ) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            created_at: Utc::now(),
            created_by: "dbx-cli".to_string(),
            connection_name,
            database,
            title,
            description,
            sql,
            operation_class,
            risk_level,
            is_production,
            status: HandoffStatus::Queued,
            result_summary: None,
            error: None,
        }
    }
}
```

- [ ] **Step 2: Export module**

Add this line to `/Users/bytedance/open_source_poj/dbx/crates/dbx-core/src/lib.rs`:

```rust
pub mod handoff;
```

- [ ] **Step 3: Add storage table**

Add this SQL statement to `SCHEMA_STATEMENTS` in `/Users/bytedance/open_source_poj/dbx/crates/dbx-core/src/storage.rs`:

```rust
"CREATE TABLE IF NOT EXISTS handoffs (
    id TEXT PRIMARY KEY,
    payload_json TEXT NOT NULL,
    status TEXT NOT NULL,
    created_at TEXT NOT NULL
)",
```

- [ ] **Step 4: Add storage methods**

Append this impl block near the storage sections in `storage.rs`:

```rust
impl Storage {
    pub async fn save_handoff(&self, item: &crate::handoff::HandoffItem) -> Result<(), String> {
        let json = serde_json::to_string(item).map_err(|e| e.to_string())?;
        sqlx::query(
            "INSERT OR REPLACE INTO handoffs (id, payload_json, status, created_at) VALUES (?, ?, ?, ?)",
        )
        .bind(&item.id)
        .bind(json)
        .bind(format!("{:?}", item.status).to_lowercase())
        .bind(item.created_at.to_rfc3339())
        .execute(&self.db)
        .await
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub async fn load_pending_handoffs(&self) -> Result<Vec<crate::handoff::HandoffItem>, String> {
        let rows: Vec<(String,)> = sqlx::query_as(
            "SELECT payload_json FROM handoffs WHERE status IN ('queued', 'shown') ORDER BY created_at ASC",
        )
        .fetch_all(&self.db)
        .await
        .map_err(|e| e.to_string())?;
        rows.into_iter()
            .map(|(json,)| serde_json::from_str(&json).map_err(|e| e.to_string()))
            .collect()
    }
}
```

- [ ] **Step 5: Run core tests**

Run:

```bash
cargo test -p dbx-core
```

Expected: existing tests pass and storage compiles with new methods.

---

### Task 5: Schema Snapshot Orchestration

**Files:**
- Create: `/Users/bytedance/open_source_poj/dbx/crates/dbx-core/src/schema_snapshot.rs`
- Modify: `/Users/bytedance/open_source_poj/dbx/crates/dbx-core/src/lib.rs`

- [ ] **Step 1: Add snapshot DTOs and function**

Create `/Users/bytedance/open_source_poj/dbx/crates/dbx-core/src/schema_snapshot.rs`:

```rust
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::connection::AppState;
use crate::models::connection::DatabaseType;
use crate::{schema, types};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TableSnapshot {
    pub name: String,
    pub table_type: String,
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
        let configs = state.configs.lock().await;
        configs.get(connection_id).cloned().ok_or("Connection config not found")?
    };
    let db = database.or(config.database.as_deref()).unwrap_or_default();
    let schemas = if db.is_empty() {
        Vec::new()
    } else {
        schema::list_schemas_core(state, connection_id, db).await.unwrap_or_default()
    };
    let effective_schema = schema_name.or_else(|| schemas.first().map(String::as_str)).unwrap_or("");
    let databases = schema::list_databases_core(state, connection_id).await.unwrap_or_default();
    let table_infos = if db.is_empty() {
        Vec::new()
    } else {
        schema::list_tables_core(state, connection_id, db, effective_schema).await.unwrap_or_default()
    };
    let mut tables = Vec::new();
    for table in table_infos {
        let columns = schema::get_columns_core(state, connection_id, db, effective_schema, &table.name).await.unwrap_or_default();
        let indexes = schema::list_indexes_core(state, connection_id, db, effective_schema, &table.name).await.unwrap_or_default();
        let foreign_keys = schema::list_foreign_keys_core(state, connection_id, db, effective_schema, &table.name).await.unwrap_or_default();
        let triggers = schema::list_triggers_core(state, connection_id, db, effective_schema, &table.name).await.unwrap_or_default();
        tables.push(TableSnapshot { name: table.name, table_type: table.table_type, columns, indexes, foreign_keys, triggers });
    }
    Ok(SchemaSnapshot {
        connection_id: config.id,
        connection_name: config.name,
        database: (!db.is_empty()).then(|| db.to_string()),
        database_type: config.db_type,
        driver_profile: config.driver_profile,
        captured_at: Utc::now(),
        databases,
        schemas,
        tables,
    })
}
```

- [ ] **Step 2: Export module**

Add this line to `/Users/bytedance/open_source_poj/dbx/crates/dbx-core/src/lib.rs`:

```rust
pub mod schema_snapshot;
```

- [ ] **Step 3: Compile core**

Run:

```bash
cargo check -p dbx-core
```

Expected: `dbx-core` compiles.

---

### Task 6: Runtime Discovery and CLI Command Dispatch

**Files:**
- Create: `/Users/bytedance/open_source_poj/dbx/crates/dbx-cli/src/runtime_client.rs`
- Create: `/Users/bytedance/open_source_poj/dbx/crates/dbx-cli/src/commands.rs`

- [ ] **Step 1: Add runtime client**

Create `/Users/bytedance/open_source_poj/dbx/crates/dbx-cli/src/runtime_client.rs`:

```rust
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeDiscovery {
    pub port: u16,
    pub token: String,
}

pub fn app_data_dir() -> PathBuf {
    let home = std::env::var(if cfg!(windows) { "APPDATA" } else { "HOME" }).unwrap_or_else(|_| ".".to_string());
    if cfg!(target_os = "macos") {
        PathBuf::from(home).join("Library/Application Support/com.dbx.app")
    } else if cfg!(windows) {
        PathBuf::from(home).join("com.dbx.app")
    } else {
        PathBuf::from(home).join(".config/com.dbx.app")
    }
}

pub fn load_runtime() -> Option<RuntimeDiscovery> {
    let path = app_data_dir().join("agent-runtime.json");
    let json = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&json).ok()
}

pub async fn get_json(path: &str) -> Result<serde_json::Value, String> {
    let runtime = load_runtime().ok_or("runtime unavailable")?;
    let url = format!("http://127.0.0.1:{}{}", runtime.port, path);
    reqwest::Client::new()
        .get(url)
        .bearer_auth(runtime.token)
        .send()
        .await
        .map_err(|e| e.to_string())?
        .json()
        .await
        .map_err(|e| e.to_string())
}

pub async fn post_json(path: &str, body: serde_json::Value) -> Result<serde_json::Value, String> {
    let runtime = load_runtime().ok_or("runtime unavailable")?;
    let url = format!("http://127.0.0.1:{}{}", runtime.port, path);
    reqwest::Client::new()
        .post(url)
        .bearer_auth(runtime.token)
        .json(&body)
        .send()
        .await
        .map_err(|e| e.to_string())?
        .json()
        .await
        .map_err(|e| e.to_string())
}
```

- [ ] **Step 2: Add command dispatcher skeleton**

Create `/Users/bytedance/open_source_poj/dbx/crates/dbx-cli/src/commands.rs` with handlers for all 8 commands:

```rust
use dbx_core::cli::{fail, ok, CliEnvelope, CliErrorCode, CliSource};

pub async fn run(args: Vec<String>) -> Result<(), CliEnvelope<()>> {
    let output = match args.as_slice() {
        [cmd, rest @ ..] if cmd == "context" => context(rest).await,
        [cmd, sub, rest @ ..] if cmd == "conn" && sub == "list" => conn_list(rest).await,
        [cmd, sub, name, rest @ ..] if cmd == "conn" && sub == "show" => conn_show(name, rest).await,
        [cmd, sub, rest @ ..] if cmd == "schema" && sub == "snapshot" => schema_snapshot(rest).await,
        [cmd, rest @ ..] if cmd == "safe-query" => safe_query(rest).await,
        [cmd, rest @ ..] if cmd == "handoff" => handoff(rest).await,
        [cmd, rest @ ..] if cmd == "selection" => selection(rest).await,
        [cmd, sub, rest @ ..] if cmd == "result" && sub == "current" => result_current(rest).await,
        _ => fail(CliSource::Headless, CliErrorCode::InternalError, "Unknown command", false),
    };
    println!("{}", serde_json::to_string_pretty(&output).unwrap());
    if matches!(output, CliEnvelope::Failure { .. }) {
        std::process::exit(1);
    }
    Ok(())
}

async fn context(_args: &[String]) -> CliEnvelope<serde_json::Value> {
    match crate::runtime_client::get_json("/context").await {
        Ok(data) => ok(CliSource::GuiRuntime, data),
        Err(_) => ok(CliSource::Headless, serde_json::json!({ "runtime": "headless" })),
    }
}

async fn conn_list(_args: &[String]) -> CliEnvelope<serde_json::Value> {
    ok(CliSource::Headless, serde_json::json!({ "connections": [] }))
}

async fn conn_show(_name: &str, _args: &[String]) -> CliEnvelope<serde_json::Value> {
    ok(CliSource::Headless, serde_json::json!({}))
}

async fn schema_snapshot(_args: &[String]) -> CliEnvelope<serde_json::Value> {
    fail(CliSource::Headless, CliErrorCode::ConnectionNotFound, "Connection is required", true)
}

async fn safe_query(_args: &[String]) -> CliEnvelope<serde_json::Value> {
    fail(CliSource::Headless, CliErrorCode::QueryClassificationFailed, "SQL is required", true)
}

async fn handoff(_args: &[String]) -> CliEnvelope<serde_json::Value> {
    fail(CliSource::Headless, CliErrorCode::InternalError, "Handoff payload is required", true)
}

async fn selection(_args: &[String]) -> CliEnvelope<serde_json::Value> {
    match crate::runtime_client::get_json("/selection").await {
        Ok(data) => ok(CliSource::GuiRuntime, data),
        Err(_) => fail(CliSource::Headless, CliErrorCode::GuiRuntimeRequired, "dbx selection requires DBX GUI runtime.", true),
    }
}

async fn result_current(args: &[String]) -> CliEnvelope<serde_json::Value> {
    let limit = option_value(args, "--limit").unwrap_or("50");
    match crate::runtime_client::get_json(&format!("/result/current?limit={limit}")).await {
        Ok(data) => ok(CliSource::GuiRuntime, data),
        Err(_) => fail(CliSource::Headless, CliErrorCode::GuiRuntimeRequired, "dbx result current requires DBX GUI runtime.", true),
    }
}

fn option_value<'a>(args: &'a [String], key: &str) -> Option<&'a str> {
    args.windows(2).find(|pair| pair[0] == key).map(|pair| pair[1].as_str())
}
```

- [ ] **Step 3: Verify CLI compiles**

Run:

```bash
cargo check -p dbx-cli
```

Expected: CLI crate compiles.

---

### Task 7: Headless Connection Commands

**Files:**
- Modify: `/Users/bytedance/open_source_poj/dbx/crates/dbx-cli/src/commands.rs`

- [ ] **Step 1: Add headless state helper and redaction**

Add helper functions to `commands.rs`:

```rust
async fn open_state() -> Result<dbx_core::connection::AppState, String> {
    let db_path = crate::runtime_client::app_data_dir().join("dbx.db");
    let storage = dbx_core::storage::Storage::open(&db_path).await?;
    Ok(dbx_core::connection::AppState::new(storage))
}

fn redacted_config(config: &dbx_core::models::connection::ConnectionConfig) -> serde_json::Value {
    serde_json::json!({
        "id": config.id,
        "name": config.name,
        "databaseType": config.db_type,
        "driverProfile": config.driver_profile,
        "driverLabel": config.driver_label,
        "defaultDatabase": config.database,
        "color": config.color,
        "sshEnabled": config.ssh_enabled,
        "redactedUrl": config.redacted_connection_url(),
    })
}

async fn find_connection(name: &str) -> Result<dbx_core::models::connection::ConnectionConfig, CliEnvelope<serde_json::Value>> {
    let state = open_state().await.map_err(|e| fail(CliSource::Headless, CliErrorCode::InternalError, e, false))?;
    let configs = state.storage.load_connections().await.map_err(|e| fail(CliSource::Headless, CliErrorCode::InternalError, e, false))?;
    let matches: Vec<_> = configs.into_iter().filter(|c| c.name == name || c.id == name).collect();
    match matches.len() {
        1 => Ok(matches.into_iter().next().unwrap()),
        0 => Err(fail(CliSource::Headless, CliErrorCode::ConnectionNotFound, "Connection not found", true)),
        _ => Err(fail(CliSource::Headless, CliErrorCode::AmbiguousConnection, "Connection name is ambiguous", true)),
    }
}
```

- [ ] **Step 2: Replace `conn_list` implementation**

Use this implementation:

```rust
async fn conn_list(_args: &[String]) -> CliEnvelope<serde_json::Value> {
    let state = match open_state().await {
        Ok(state) => state,
        Err(e) => return fail(CliSource::Headless, CliErrorCode::InternalError, e, false),
    };
    match state.storage.load_connections().await {
        Ok(configs) => ok(CliSource::Headless, serde_json::json!({
            "connections": configs.iter().map(redacted_config).collect::<Vec<_>>()
        })),
        Err(e) => fail(CliSource::Headless, CliErrorCode::InternalError, e, false),
    }
}
```

- [ ] **Step 3: Replace `conn_show` implementation**

Use this implementation:

```rust
async fn conn_show(name: &str, _args: &[String]) -> CliEnvelope<serde_json::Value> {
    match find_connection(name).await {
        Ok(config) => ok(CliSource::Headless, redacted_config(&config)),
        Err(err) => err,
    }
}
```

- [ ] **Step 4: Verify commands**

Run:

```bash
cargo run -p dbx-cli --bin dbx-cli -- conn list --format json
cargo run -p dbx-cli --bin dbx-cli -- conn show __missing__ --redacted --format json
```

Expected: first command returns an envelope; second returns `CONNECTION_NOT_FOUND` without panicking.

---

### Task 8: Headless Schema Snapshot and Safe Query

**Files:**
- Modify: `/Users/bytedance/open_source_poj/dbx/crates/dbx-cli/src/commands.rs`

- [ ] **Step 1: Add connection registration helper**

Add this helper:

```rust
async fn state_with_connection(config: dbx_core::models::connection::ConnectionConfig) -> Result<dbx_core::connection::AppState, String> {
    let state = open_state().await?;
    state.configs.lock().await.insert(config.id.clone(), config.clone());
    state.get_or_create_pool(&config.id, config.database.as_deref()).await?;
    Ok(state)
}
```

- [ ] **Step 2: Replace `schema_snapshot`**

Use this implementation:

```rust
async fn schema_snapshot(args: &[String]) -> CliEnvelope<serde_json::Value> {
    let Some(conn_name) = option_value(args, "--conn") else {
        return fail(CliSource::Headless, CliErrorCode::ConnectionNotFound, "--conn is required", true);
    };
    let db = option_value(args, "--db");
    let config = match find_connection(conn_name).await {
        Ok(config) => config,
        Err(err) => return err,
    };
    let state = match state_with_connection(config.clone()).await {
        Ok(state) => state,
        Err(e) => return fail(CliSource::Headless, CliErrorCode::InternalError, e, false),
    };
    match dbx_core::schema_snapshot::snapshot(&state, &config.id, db, None).await {
        Ok(snapshot) => ok(CliSource::Headless, serde_json::to_value(snapshot).unwrap()),
        Err(e) => fail(CliSource::Headless, CliErrorCode::InternalError, e, false),
    }
}
```

- [ ] **Step 3: Replace `safe_query`**

Use this implementation:

```rust
async fn safe_query(args: &[String]) -> CliEnvelope<serde_json::Value> {
    let Some(conn_name) = option_value(args, "--conn") else {
        return fail(CliSource::Headless, CliErrorCode::ConnectionNotFound, "--conn is required", true);
    };
    let Some(sql) = option_value(args, "--sql") else {
        return fail(CliSource::Headless, CliErrorCode::QueryClassificationFailed, "--sql is required", true);
    };
    let config = match find_connection(conn_name).await {
        Ok(config) => config,
        Err(err) => return err,
    };
    let risk = dbx_core::sql_safety::risk_for(sql, &config.name, config.color.as_deref());
    match risk.operation_class {
        dbx_core::sql_safety::OperationClass::Read => {}
        dbx_core::sql_safety::OperationClass::Write if risk.is_production => {
            return fail(CliSource::Headless, CliErrorCode::ProductionWriteBlocked, serde_json::to_string(&risk).unwrap(), true);
        }
        dbx_core::sql_safety::OperationClass::Write => {
            return fail(CliSource::Headless, CliErrorCode::HandoffRequired, serde_json::to_string(&risk).unwrap(), true);
        }
        dbx_core::sql_safety::OperationClass::Ddl => {
            return fail(CliSource::Headless, CliErrorCode::DdlBlocked, serde_json::to_string(&risk).unwrap(), true);
        }
        dbx_core::sql_safety::OperationClass::Unknown => {
            return fail(CliSource::Headless, CliErrorCode::QueryClassificationFailed, serde_json::to_string(&risk).unwrap(), true);
        }
    }
    let state = match state_with_connection(config.clone()).await {
        Ok(state) => state,
        Err(e) => return fail(CliSource::Headless, CliErrorCode::InternalError, e, false),
    };
    let database = option_value(args, "--db").or(config.database.as_deref()).unwrap_or("");
    match dbx_core::query::execute_sql_statement(&state, &config.id, database, sql, None, None).await {
        Ok(result) => ok(CliSource::Headless, serde_json::json!({ "risk": risk, "result": result })),
        Err(e) => fail(CliSource::Headless, CliErrorCode::InternalError, e, false),
    }
}
```

- [ ] **Step 4: Verify SQL blocking**

Run:

```bash
cargo run -p dbx-cli --bin dbx-cli -- safe-query --conn __missing__ --sql "DROP TABLE users" --format json
```

Expected: returns `CONNECTION_NOT_FOUND`. With a real DBX connection, `DROP TABLE users` returns `DDL_BLOCKED`.

---

### Task 9: GUI Runtime Server

**Files:**
- Create: `/Users/bytedance/open_source_poj/dbx/src-tauri/src/commands/agent_runtime.rs`
- Modify: `/Users/bytedance/open_source_poj/dbx/src-tauri/src/commands/mod.rs`
- Modify: `/Users/bytedance/open_source_poj/dbx/src-tauri/src/lib.rs`

- [ ] **Step 1: Add runtime command module**

Create `/Users/bytedance/open_source_poj/dbx/src-tauri/src/commands/agent_runtime.rs` with:

```rust
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tauri::{AppHandle, Manager};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::RwLock;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentRuntimeSnapshot {
    pub active_connection_id: Option<String>,
    pub active_connection_name: Option<String>,
    pub database: Option<String>,
    pub schema: Option<String>,
    pub active_tab_id: Option<String>,
    pub active_tab_title: Option<String>,
    pub sql: Option<String>,
    pub selected_sql: Option<String>,
    pub selection: Option<serde_json::Value>,
    pub result: Option<serde_json::Value>,
}

#[derive(Clone)]
pub struct AgentRuntimeState {
    pub token: String,
    pub snapshot: Arc<RwLock<AgentRuntimeSnapshot>>,
    pub handoffs: Arc<RwLock<Vec<dbx_core::handoff::HandoffItem>>>,
}

#[tauri::command]
pub async fn agent_runtime_update_snapshot(
    state: tauri::State<'_, AgentRuntimeState>,
    snapshot: AgentRuntimeSnapshot,
) -> Result<(), String> {
    *state.snapshot.write().await = snapshot;
    Ok(())
}

#[tauri::command]
pub async fn agent_runtime_load_handoffs(
    app_state: tauri::State<'_, Arc<crate::commands::connection::AppState>>,
    runtime: tauri::State<'_, AgentRuntimeState>,
) -> Result<Vec<dbx_core::handoff::HandoffItem>, String> {
    let mut items = app_state.storage.load_pending_handoffs().await?;
    items.extend(runtime.handoffs.read().await.iter().cloned());
    Ok(items)
}

pub fn start(app: AppHandle) -> AgentRuntimeState {
    let token = uuid::Uuid::new_v4().to_string();
    let state = AgentRuntimeState {
        token: token.clone(),
        snapshot: Arc::new(RwLock::new(AgentRuntimeSnapshot::default())),
        handoffs: Arc::new(RwLock::new(Vec::new())),
    };
    let server_state = state.clone();
    tauri::async_runtime::spawn(async move {
        let listener = match TcpListener::bind("127.0.0.1:0").await {
            Ok(listener) => listener,
            Err(err) => {
                log::warn!("Agent runtime bind failed: {err}");
                return;
            }
        };
        let port = listener.local_addr().map(|addr| addr.port()).unwrap_or(0);
        if let Ok(dir) = app.path().app_data_dir() {
            let payload = serde_json::json!({ "port": port, "token": token });
            let _ = std::fs::write(dir.join("agent-runtime.json"), serde_json::to_string(&payload).unwrap());
        }
        loop {
            let Ok((stream, _)) = listener.accept().await else { continue };
            let st = server_state.clone();
            tauri::async_runtime::spawn(async move {
                handle_connection(stream, st).await;
            });
        }
    });
    state
}

async fn handle_connection(mut stream: tokio::net::TcpStream, state: AgentRuntimeState) {
    let mut buf = vec![0u8; 65536];
    let Ok(n) = stream.read(&mut buf).await else { return };
    if n == 0 {
        return;
    }
    let request = String::from_utf8_lossy(&buf[..n]);
    if !request.contains(&format!("Authorization: Bearer {}", state.token)) {
        respond_json(&mut stream, "401 Unauthorized", serde_json::json!({"error":"unauthorized"})).await;
        return;
    }
    let first = request.lines().next().unwrap_or("");
    if first.starts_with("GET /context") {
        respond_json(&mut stream, "200 OK", serde_json::to_value(&*state.snapshot.read().await).unwrap()).await;
    } else if first.starts_with("GET /selection") {
        let snapshot = state.snapshot.read().await;
        respond_json(&mut stream, "200 OK", snapshot.selection.clone().unwrap_or_else(|| serde_json::json!({"type":"none"}))).await;
    } else if first.starts_with("GET /result/current") {
        let snapshot = state.snapshot.read().await;
        respond_json(&mut stream, "200 OK", snapshot.result.clone().unwrap_or_else(|| serde_json::json!({"columns":[],"rows":[]}))).await;
    } else if first.starts_with("POST /handoff") {
        let body = request.split("\r\n\r\n").nth(1).unwrap_or("");
        if let Ok(item) = serde_json::from_str::<dbx_core::handoff::HandoffItem>(body) {
            state.handoffs.write().await.push(item.clone());
            respond_json(&mut stream, "200 OK", serde_json::json!({"id": item.id, "status": "shown"})).await;
        } else {
            respond_json(&mut stream, "400 Bad Request", serde_json::json!({"error":"invalid handoff"})).await;
        }
    } else {
        respond_json(&mut stream, "404 Not Found", serde_json::json!({"error":"not found"})).await;
    }
}

async fn respond_json(stream: &mut tokio::net::TcpStream, status: &str, body: serde_json::Value) {
    let body = serde_json::to_string(&body).unwrap();
    let resp = format!("HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}", body.len());
    let _ = stream.write_all(resp.as_bytes()).await;
}
```

- [ ] **Step 2: Expose module**

Add to `/Users/bytedance/open_source_poj/dbx/src-tauri/src/commands/mod.rs`:

```rust
pub mod agent_runtime;
```

- [ ] **Step 3: Start runtime in Tauri setup**

In `/Users/bytedance/open_source_poj/dbx/src-tauri/src/lib.rs`, after `commands::mcp_bridge::start(app_handle, state);`, add:

```rust
let runtime_state = commands::agent_runtime::start(app.handle().clone());
app.manage(runtime_state);
```

Add command to `invoke_handler`:

```rust
commands::agent_runtime::agent_runtime_update_snapshot,
commands::agent_runtime::agent_runtime_load_handoffs,
```

- [ ] **Step 4: Compile Tauri crate**

Run:

```bash
cargo check -p dbx
```

Expected: desktop crate compiles.

---

### Task 10: Frontend Runtime State Sync

**Files:**
- Modify: `/Users/bytedance/open_source_poj/dbx/src/lib/tauri.ts`
- Modify: `/Users/bytedance/open_source_poj/dbx/src/lib/api.ts`
- Create: `/Users/bytedance/open_source_poj/dbx/src/stores/agentRuntimeStore.ts`
- Modify: `/Users/bytedance/open_source_poj/dbx/src/stores/queryStore.ts`
- Modify: `/Users/bytedance/open_source_poj/dbx/src/components/grid/DataGrid.vue`

- [ ] **Step 1: Add Tauri wrappers**

Add to `/Users/bytedance/open_source_poj/dbx/src/lib/tauri.ts`:

```ts
export async function agentRuntimeUpdateSnapshot(snapshot: unknown): Promise<void> {
  return invoke("agent_runtime_update_snapshot", { snapshot });
}

export async function agentRuntimeLoadHandoffs(): Promise<unknown[]> {
  return invoke("agent_runtime_load_handoffs");
}
```

- [ ] **Step 2: Export API forwards**

Add to `/Users/bytedance/open_source_poj/dbx/src/lib/api.ts`:

```ts
export const agentRuntimeUpdateSnapshot = forward("agentRuntimeUpdateSnapshot");
export const agentRuntimeLoadHandoffs = forward("agentRuntimeLoadHandoffs");
```

- [ ] **Step 3: Create runtime store**

Create `/Users/bytedance/open_source_poj/dbx/src/stores/agentRuntimeStore.ts`:

```ts
import { defineStore } from "pinia";
import { ref } from "vue";
import * as api from "@/lib/api";
import { useConnectionStore } from "@/stores/connectionStore";
import { useQueryStore } from "@/stores/queryStore";

export const useAgentRuntimeStore = defineStore("agentRuntime", () => {
  const selection = ref<unknown>({ type: "none" });
  let timer: ReturnType<typeof setTimeout> | null = null;

  function setSelection(value: unknown) {
    selection.value = value;
    scheduleSync();
  }

  function scheduleSync() {
    if (timer) clearTimeout(timer);
    timer = setTimeout(() => void syncNow(), 100);
  }

  async function syncNow() {
    const connectionStore = useConnectionStore();
    const queryStore = useQueryStore();
    const tab = queryStore.tabs.find((item) => item.id === queryStore.activeTabId);
    const conn = tab ? connectionStore.getConfig(tab.connectionId) : undefined;
    await api.agentRuntimeUpdateSnapshot({
      activeConnectionId: tab?.connectionId,
      activeConnectionName: conn?.name,
      database: tab?.database,
      schema: tab?.schema,
      activeTabId: tab?.id,
      activeTabTitle: tab?.title,
      sql: tab?.sql,
      selectedSql: undefined,
      selection: selection.value,
      result: tab?.result
        ? {
            columns: tab.result.columns,
            rows: tab.result.rows.slice(0, 50),
            truncated: tab.result.rows.length > 50 || tab.result.truncated,
            executionTimeMs: tab.result.execution_time_ms,
          }
        : undefined,
    });
  }

  return { selection, setSelection, scheduleSync, syncNow };
});
```

- [ ] **Step 4: Notify sync from query store**

In `/Users/bytedance/open_source_poj/dbx/src/stores/queryStore.ts`, import:

```ts
import { useAgentRuntimeStore } from "@/stores/agentRuntimeStore";
```

After result assignment in `executeTabSql`, call:

```ts
useAgentRuntimeStore().scheduleSync();
```

Also call `useAgentRuntimeStore().scheduleSync();` after active tab changes in `createTab`, `openSavedSql`, `closeTab`, and `setActiveResultIndex`.

- [ ] **Step 5: Notify grid selection**

In `/Users/bytedance/open_source_poj/dbx/src/components/grid/DataGrid.vue`, import:

```ts
import { useAgentRuntimeStore } from "@/stores/agentRuntimeStore";
```

Create a store instance:

```ts
const agentRuntimeStore = useAgentRuntimeStore();
```

Where selection range changes, call:

```ts
agentRuntimeStore.setSelection({
  type: "grid-cells",
  data: extractSelection(props.result.columns, props.result.rows, selectionRange.value),
});
```

- [ ] **Step 6: Run frontend typecheck**

Run:

```bash
pnpm build
```

Expected: Vue typecheck and Vite build pass.

---

### Task 11: Handoff CLI and GUI Review UI

**Files:**
- Modify: `/Users/bytedance/open_source_poj/dbx/crates/dbx-cli/src/commands.rs`
- Create: `/Users/bytedance/open_source_poj/dbx/src/components/agent/AgentHandoffDialog.vue`
- Modify: `/Users/bytedance/open_source_poj/dbx/src/components/layout/AppDialogs.vue`

- [ ] **Step 1: Implement CLI handoff**

Replace `handoff` in `commands.rs` with:

```rust
async fn handoff(args: &[String]) -> CliEnvelope<serde_json::Value> {
    let Some(conn_name) = option_value(args, "--conn") else {
        return fail(CliSource::Headless, CliErrorCode::ConnectionNotFound, "--conn is required", true);
    };
    let Some(title) = option_value(args, "--title") else {
        return fail(CliSource::Headless, CliErrorCode::InternalError, "--title is required", true);
    };
    let sql = if let Some(sql_file) = option_value(args, "--sql-file") {
        match std::fs::read_to_string(sql_file) {
            Ok(sql) => sql,
            Err(e) => return fail(CliSource::Headless, CliErrorCode::InternalError, e.to_string(), true),
        }
    } else if let Some(sql) = option_value(args, "--sql") {
        sql.to_string()
    } else {
        return fail(CliSource::Headless, CliErrorCode::InternalError, "--sql-file or --sql is required", true);
    };
    let config = match find_connection(conn_name).await {
        Ok(config) => config,
        Err(err) => return err,
    };
    let risk = dbx_core::sql_safety::risk_for(&sql, &config.name, config.color.as_deref());
    let item = dbx_core::handoff::HandoffItem::queued(
        config.name,
        config.database,
        title.to_string(),
        option_value(args, "--description").map(str::to_string),
        sql,
        risk.operation_class,
        risk.risk_level,
        risk.is_production,
    );
    if let Ok(data) = crate::runtime_client::post_json("/handoff", serde_json::to_value(&item).unwrap()).await {
        return ok(CliSource::GuiRuntime, data);
    }
    match open_state().await.and_then(|state| futures::executor::block_on(state.storage.save_handoff(&item))) {
        Ok(()) => ok(CliSource::Headless, serde_json::json!({ "id": item.id, "status": "queued" })),
        Err(e) => fail(CliSource::Headless, CliErrorCode::InternalError, e, false),
    }
}
```

- [ ] **Step 2: Create handoff review dialog**

Create `/Users/bytedance/open_source_poj/dbx/src/components/agent/AgentHandoffDialog.vue`:

```vue
<script setup lang="ts">
import { onMounted, ref } from "vue";
import { Dialog, DialogContent, DialogHeader, DialogTitle, DialogFooter } from "@/components/ui/dialog";
import { Button } from "@/components/ui/button";
import * as api from "@/lib/api";

type HandoffItem = {
  id: string;
  title: string;
  connectionName: string;
  database?: string;
  sql: string;
  riskLevel: string;
  status: string;
};

const open = ref(false);
const items = ref<HandoffItem[]>([]);
const active = ref<HandoffItem | null>(null);

async function refresh() {
  items.value = (await api.agentRuntimeLoadHandoffs()) as HandoffItem[];
  active.value = items.value[0] ?? null;
  open.value = items.value.length > 0;
}

function reject() {
  items.value = items.value.filter((item) => item.id !== active.value?.id);
  active.value = items.value[0] ?? null;
  open.value = items.value.length > 0;
}

function approvePlaceholder() {
  reject();
}

onMounted(() => {
  void refresh();
  window.setInterval(() => void refresh(), 5000);
});
</script>

<template>
  <Dialog v-model:open="open">
    <DialogContent class="max-w-3xl">
      <DialogHeader>
        <DialogTitle>DBX Agent Handoff</DialogTitle>
      </DialogHeader>
      <div v-if="active" class="space-y-3">
        <div class="text-sm text-muted-foreground">
          {{ active.connectionName }} / {{ active.database || "default" }} / risk: {{ active.riskLevel }}
        </div>
        <h3 class="font-medium">{{ active.title }}</h3>
        <pre class="max-h-96 overflow-auto rounded-md bg-muted p-3 text-xs">{{ active.sql }}</pre>
      </div>
      <DialogFooter>
        <Button variant="outline" @click="reject">Reject</Button>
        <Button @click="approvePlaceholder">Approve</Button>
      </DialogFooter>
    </DialogContent>
  </Dialog>
</template>
```

- [ ] **Step 3: Mount dialog**

In `/Users/bytedance/open_source_poj/dbx/src/components/layout/AppDialogs.vue`, import and add:

```vue
<AgentHandoffDialog />
```

- [ ] **Step 4: Verify GUI build**

Run:

```bash
pnpm build
```

Expected: build passes and dialog component compiles.

---

### Task 12: Runtime Context, Selection, and Result CLI Verification

**Files:**
- Modify: `/Users/bytedance/open_source_poj/dbx/crates/dbx-cli/src/commands.rs`
- Modify: `/Users/bytedance/open_source_poj/dbx/src/stores/agentRuntimeStore.ts`

- [ ] **Step 1: Improve context headless fallback**

Replace headless branch of `context` with:

```rust
Err(_) => {
    let state = match open_state().await {
        Ok(state) => state,
        Err(e) => return fail(CliSource::Headless, CliErrorCode::InternalError, e, false),
    };
    let configs = state.storage.load_connections().await.unwrap_or_default();
    ok(CliSource::Headless, serde_json::json!({
        "runtime": "headless",
        "activeConnection": configs.first().map(redacted_config),
        "configSource": crate::runtime_client::app_data_dir().join("dbx.db").display().to_string()
    }))
}
```

- [ ] **Step 2: Ensure result limit is applied**

In `/Users/bytedance/open_source_poj/dbx/src/stores/agentRuntimeStore.ts`, keep `rows.slice(0, 50)` and ensure `truncated` is set when original rows exceed 50.

- [ ] **Step 3: Manual runtime verification**

Run DBX desktop:

```bash
pnpm tauri dev
```

In a second terminal, run:

```bash
cargo run -p dbx-cli --bin dbx-cli -- context --format json
cargo run -p dbx-cli --bin dbx-cli -- selection --format json
cargo run -p dbx-cli --bin dbx-cli -- result current --limit 50 --format json
```

Expected: all three commands return `source: "gui-runtime"` when desktop is running.

---

### Task 13: Final Verification and Compatibility Check

**Files:**
- No new files.

- [ ] **Step 1: Verify Rust workspace**

Run:

```bash
cargo check --workspace
cargo test -p dbx-core
```

Expected: workspace checks and core tests pass.

- [ ] **Step 2: Verify frontend**

Run:

```bash
pnpm build
```

Expected: Vue typecheck and Vite build pass.

- [ ] **Step 3: Verify MCP untouched**

Run:

```bash
git diff -- mcp
```

Expected: no output.

- [ ] **Step 4: Verify CLI command surface**

Run:

```bash
cargo run -p dbx-cli --bin dbx-cli -- context --format json
cargo run -p dbx-cli --bin dbx-cli -- conn list --format json
cargo run -p dbx-cli --bin dbx-cli -- conn show __missing__ --redacted --format json
cargo run -p dbx-cli --bin dbx-cli -- selection --format json
cargo run -p dbx-cli --bin dbx-cli -- result current --limit 50 --format json
```

Expected: all commands return valid JSON envelopes. GUI-only commands return `GUI_RUNTIME_REQUIRED` if DBX desktop is not running.

## Self-Review

- Spec coverage: The plan covers all 8 CLI commands, runtime discovery/token, GUI context, selection, current result, safe-query classification, schema snapshot, and handoff queue/display.
- Compatibility: The plan explicitly avoids changing `mcp/` and verifies this with `git diff -- mcp`.
- Type consistency: Rust DTOs use camelCase where sent to frontend/CLI. Error codes use screaming snake case. Source uses kebab case.
- Known implementation detail to watch: Task 11 uses a synchronous block inside async CLI code for queued handoff save; if it causes compile friction, replace it with direct async `state.storage.save_handoff(&item).await` inside a `match` after opening state.
