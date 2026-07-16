# @dbx-app/cli

Command-line interface for DBX — list connections, inspect schema, run safe queries, and mirror MCP server capabilities from the terminal.

## Install

```bash
npm install -g @dbx-app/cli
```

Or use the `dbx` shim from a global npm prefix (see DBX docs).

## Commands

| Command | Description |
|---------|-------------|
| `dbx doctor` | Check DBX data directory, connection store, and desktop bridge |
| `dbx capabilities` | List database types supported via direct query vs desktop bridge |
| `dbx connections list` | List configured connections |
| `dbx connections add` | Add a connection (inline proxy or saved proxy profile) |
| `dbx proxies list` | List saved proxy tunnel profiles from DBX Settings > Tunnels |
| `dbx stats` | Database status overview from system catalog views |
| `dbx report` | Full database report: summary, tables, column comments, indexes |
| `dbx schema list` | List tables and views |
| `dbx schema describe` | Show column definitions for a table |
| `dbx query` | Execute a SQL query (read-only by default) |
| `dbx context` | Compact schema context for writing SQL |
| `dbx open` | Open a table in the DBX desktop app (requires DBX running) |

## `dbx connections add`

Required flags: `--name`, `--type`, `--host`. Optional: `--port`, `--username`, `--password`, `--database`, `--ssl`, `--driver-profile`.

### Inline proxy (SOCKS5 / HTTP)

```bash
dbx connections add --name prod --type postgres --host db.example.com --port 5432 \
  --proxy --proxy-type socks5 --proxy-host 127.0.0.1 --proxy-port 1080
```

### Saved proxy profile (Settings > Tunnels)

```bash
dbx proxies list
dbx connections add --name prod --type postgres --host db.example.com --port 5432 \
  --proxy-profile-name "Office SOCKS5"
```

Inline proxy flags and profile reference flags are **mutually exclusive**. Specify either `--proxy-profile-id` or `--proxy-profile-name`, not both.

## Connection references

List output includes a `#` column (1-based index). Use a single index or a **range** (CLI only, non-interactive):

```bash
dbx connections list
dbx stats 1              # first connection
dbx stats 1-15           # connections #1 through #15 (sequential)
dbx stats 23-50          # any valid index range (sequential by default)
dbx stats 23-50 --parallel   # same range, up to 15 concurrent (default)
dbx stats 23-50 -P 3         # max 3 concurrent
dbx report 3..5          # connections #3, #4, #5
dbx query 2 "select 1"
```

Range syntax: `1-15`, `1..15`, `1:15`, `#1-#15`, `23-50`. **No span cap** — any valid start–end range is allowed. Use `--parallel` to limit simultaneous connections (default **15**), not range size.

**Parallel batch:** add `--parallel` or `-P` to run multiple connections concurrently (default concurrency **15**). Use `-P N` to set the limit (capped at batch size). Without the flag, connections run **sequentially** (one at a time). Applies to `stats`, `report`, `query`, `schema list`, `schema describe`, `context`, and `open`. Stderr progress lines are prefixed with `[#N]` in parallel mode. stdout/JSON results stay in original index order, separated by `---`.

## `dbx stats`

Catalog-based overview (table metadata, size/row estimates) matching MCP `dbx_get_database_stats`:

```bash
dbx stats my-postgres --schema public
dbx stats my-mysql --json
```

Supports MySQL/MariaDB family, PostgreSQL family, SQLite/rqlite, MongoDB (collStats), Redis (INFO/DBSIZE), and generic `information_schema` fallback for other SQL engines.

## `dbx report`

Comprehensive catalog-based report matching MCP `dbx_get_database_report`:

```bash
dbx report my-postgres --schema public
dbx report 1 --json
```

Report sections: Database Summary, Tables (sorted by row estimate), Column Comments, Indexes. All instant catalog data — no `COUNT(*)` queries.

## Environment

| Variable | Description |
|----------|-------------|
| `DBX_CONNECTION` | Default connection name for `query`, `context`, and `stats` |
| `DBX_MCP_ALLOW_WRITES` | Allow write SQL in `dbx query` when set to `1` or `true` |
| `DBX_MCP_ALLOW_DANGEROUS_SQL` | Allow dangerous SQL patterns (requires writes enabled) |
| `DBX_WEB_URL` | Use DBX Web backend instead of local SQLite store |

## Output formats

- Default: Markdown-style tables on stdout
- `--json` or `--format json`: JSON output
- `--format csv`: CSV (where supported)

Errors go to stderr with exit code `1`.

---

# @dbx-app/cli（中文）

DBX 命令行工具 — 在终端中列出连接、查看 schema、执行安全查询，并与 MCP Server 能力对齐。

## 命令

| 命令 | 说明 |
|------|------|
| `dbx connections add` | 添加连接（内联代理或引用已保存代理配置） |
| `dbx proxies list` | 列出 DBX **设置 > 隧道** 中已保存的代理配置 |
| `dbx stats` | 从系统目录视图获取数据库状态概览（对应 MCP `dbx_get_database_stats`） |
| `dbx report` | 完整数据库报告：摘要、表、列注释、索引（对应 MCP `dbx_get_database_report`） |

### 添加连接 — 内联代理

```bash
dbx connections add --name prod --type postgres --host db.example.com --port 5432 \
  --proxy --proxy-host 127.0.0.1 --proxy-port 1080
```

### 添加连接 — 引用已保存代理

```bash
dbx proxies list
dbx connections add --name prod --type postgres --host db.example.com --port 5432 \
  --proxy-profile-id "<uuid>"
```

内联 `proxy_*` 参数与 `--proxy-profile-id` / `--proxy-profile-name` **不可混用**。

### 连接序号与范围（非交互 CLI）

`dbx connections list` 的 `#` 列表示 1-based 序号。支持单个序号或范围批量：

```bash
dbx stats 1-15           # 依次对 #1–#15 执行 stats
dbx stats 23-35 --parallel   # 并行，默认最多 5 个并发
dbx stats 23-35 -P 3         # 最多 3 个并发
dbx report 3..5          # #3、#4、#5
dbx query 1 "select 1"   # 单连接不变
```

范围语法：`1-15`、`1..15`、`1:15`、`#1-#15`。单次最多 15 个连接（按范围跨度计，结束序号无上限）。

**并行批量：** 加 `--parallel` 或 `-P` 可并发执行多个连接（默认并发数 **5**）；`-P N` 指定上限（不超过批量大小）。不加标志时仍为**顺序**执行。适用于 `stats`、`report`、`query`、`schema list`、`schema describe`、`context`、`open`。并行模式下 stderr 进度行带 `[#N]` 前缀；stdout/JSON 结果仍按原序号排列，以 `---` 分隔。

### 数据库状态概览

```bash
dbx stats my-postgres --schema public
```
