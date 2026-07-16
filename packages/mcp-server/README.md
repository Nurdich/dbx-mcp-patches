# DBX MCP Server

MCP server for [DBX](https://github.com/t8y2/dbx) — lets AI agents (Claude Code, Cursor, etc.) query your databases using connections already configured in DBX.

[中文](#中文说明) | English

## Features

- **Zero config** — Automatically reads your DBX connections (including passwords from system keyring)
- **11 tools** — List/add/remove connections, list saved proxies, list tables, describe table, get database stats, get schema context, execute SQL, execute Redis commands, open table in DBX UI
- **Connection pooling** — Reuses database connections across queries
- **Direct execution** — PostgreSQL, MySQL, SQLite, and compatible databases (Doris, StarRocks, etc.) can run without opening DBX
- **Writes enabled by default** — regular `INSERT` / `UPDATE` / `DELETE` statements work out of the box, while dangerous SQL stays blocked unless explicitly enabled
- **DBX UI integration** — Open tables directly in the DBX desktop app from your AI agent

## Quick Start

### 1. Install

```bash
npm install -g @dbx-app/mcp-server
```

Or run directly:

```bash
npx @dbx-app/mcp-server
```

### 2. Configure Claude Code

Add to your project's `.mcp.json`:

```json
{
  "mcpServers": {
    "dbx": {
      "command": "dbx-mcp-server"
    }
  }
}
```

For Windows portable builds, set `DBX_DATA_DIR` to the portable data directory that contains `dbx.db`:

```json
{
  "mcpServers": {
    "dbx": {
      "command": "dbx-mcp-server",
      "env": {
        "DBX_DATA_DIR": "D:\\DBX_x64-portable\\data"
      }
    }
  }
}
```

Or for development (from source):

```json
{
  "mcpServers": {
    "dbx": {
      "command": "npx",
      "args": ["tsx", "packages/mcp-server/src/index.ts"],
      "cwd": "/path/to/dbx"
    }
  }
}
```

### 3. Use

In Claude Code, just ask:

- "List my database connections"
- "Show the tables in my local-pg connection"
- "Describe the users table"
- "Query the average salary from employees"
- "Open the orders table in DBX"

## CLI

For terminal, script, and Codex workflows, install the dedicated CLI package:

```bash
npm install -g @dbx-app/cli
dbx connections list --json
dbx query local "select 1" --json
```

See the [DBX CLI README](../cli/README.md) for command details.

### List indexes (`#` column)

Connection, proxy, and table listings include a **`#`** column — a **1-based row index** in the current list output order (same order used when resolving numeric references).

You can pass that index instead of a UUID or name:

| Input examples | Resolves to |
| -------------- | ----------- |
| `1`, `#2` | Item at row 1 or 2 from the latest list of that category |
| Exact UUID / name | Still preferred when present (checked before index) |

**MCP:** use `connection_id`, `connection_name`, `proxy_profile_id`, or `proxy_profile_name` with a numeric value.

**CLI:** use the connection argument positionally, e.g. `dbx stats 1`, `dbx query 2 "select 1"`, or `--proxy-profile-id 1`.

Lists reload data on each call; index `N` always means “the Nth row in the current list,” not a persisted database ID.

## Tools

| Tool                        | Description                                          |
| --------------------------- | ---------------------------------------------------- |
| `dbx_list_connections`      | List all database connections configured in DBX      |
| `dbx_add_connection`        | Add a new database connection                        |
| `dbx_list_proxies`          | List saved proxy tunnel profiles from DBX Settings   |
| `dbx_remove_connection`     | Remove a database connection                         |
| `dbx_list_tables`           | List tables and views for a connection               |
| `dbx_describe_table`        | Get column definitions for a table                   |
| `dbx_get_database_stats`    | Database status overview from system catalog views   |
| `dbx_get_database_report`   | Full database report: summary, tables, comments, indexes |
| `dbx_get_schema_context`    | Get compact table and column context for writing SQL |
| `dbx_execute_query`         | Execute a SQL query (max 100 rows)                   |
| `dbx_execute_redis_command` | Execute a Redis command on a Redis connection        |
| `dbx_open_table`            | Open a table in DBX desktop app UI                   |

### `dbx_add_connection` Parameters

| Parameter         | Type    | Required | Default  | Description                                              |
| ----------------- | ------- | -------- | -------- | -------------------------------------------------------- |
| `name`            | string  | yes      | —        | Connection name                                          |
| `db_type`         | string  | yes      | —        | Database type (e.g. `mysql`, `postgresql`, `redis`)      |
| `host`            | string  | yes      | —        | Database host; for cloudflare-d1, use the Account ID     |
| `port`            | number  | no       | —        | Database port (defaults vary by db_type)                 |
| `username`        | string  | no       | `""`     | Username                                                 |
| `password`        | string  | no       | `""`     | Password; for cloudflare-d1, use the API Token         |
| `database`        | string  | no       | —        | Default database name; for cloudflare-d1, use the D1 ID  |
| `ssl`             | boolean | no       | `false`  | Enable SSL                                               |
| `driver_profile`  | string  | no       | —        | Driver profile (e.g. `gbase8a`, `gbase8s`)              |
| `proxy_enabled`   | boolean | no       | `false`  | Enable SOCKS5 or HTTP proxy tunnel                       |
| `proxy_type`      | enum    | no       | `socks5` | Proxy protocol: `socks5` or `http`                       |
| `proxy_host`      | string  | when proxy enabled | — | Proxy server host                             |
| `proxy_port`      | number  | no       | `1080`   | Proxy server port (1–65535)                              |
| `proxy_username`      | string  | no       | —        | Proxy authentication username                            |
| `proxy_password`      | string  | no       | —        | Proxy authentication password                            |
| `proxy_profile_id`    | string  | no       | —        | Saved proxy profile ID, name, or list index `#` from `dbx_list_proxies` |
| `proxy_profile_name`  | string  | no       | —        | Saved proxy profile name or list index `#` (alternative to ID)        |

**Proxy modes (mutually exclusive):**

1. **Inline** — set `proxy_enabled=true` and provide `proxy_host` (plus optional `proxy_type`, `proxy_port`, credentials). Stored via legacy `proxy_*` fields converted to a `transport_layers` proxy entry.
2. **Saved profile** — set `proxy_profile_id` or `proxy_profile_name` to reference a tunnel profile from DBX **Settings > Tunnels**. The connection stores a `transport_layers` stub with `profile_id`; DBX resolves the full proxy config at connect time.

You cannot mix inline proxy settings with `proxy_profile_id` / `proxy_profile_name`. Specify only one lookup key for saved profiles (ID or name, not both). Use `dbx_list_proxies` to discover available profiles.

SSH tunnel parameters are not yet exposed by this MCP tool.

### `dbx_get_database_stats`

Returns a compact markdown overview suitable for AI agents. All metrics come from database system catalogs — no manual `COUNT(*)` or cached counters.

| Parameter         | Type   | Required | Description                                              |
| ----------------- | ------ | -------- | -------------------------------------------------------- |
| `connection_id`   | string | no       | Connection UUID, or list index `#` from `dbx_list_connections` |
| `connection_name` | string | no       | Connection name, or list index `#` from `dbx_list_connections` |
| `database`        | string | no       | Database name (Dameng: also accepted as schema alias)  |
| `schema`          | string | no       | Schema name (default: `public` for PostgreSQL, `dbo` for SQL Server) |

**Query strategy by database type:**

| Type | Catalog source |
| ---- | -------------- |
| MySQL / MariaDB / Doris / StarRocks / Manticore | `information_schema.TABLES`, `information_schema.SCHEMATA` |
| PostgreSQL family | `information_schema.tables` + `pg_catalog` + `pg_stat_user_tables` |
| SQLite / rqlite | `sqlite_master` |
| MongoDB | `collStats` per collection (up to 50) |
| Redis | `INFO` + `DBSIZE` |
| Other SQL (via bridge) | `information_schema.tables` best-effort fallback |

Output columns: Name, Type, Engine, Rows/Docs (estimate), Data size, Index size, Total size, Comment.

### `dbx_get_database_report`

Returns a structured markdown report for AI agents and humans. All data comes from system catalogs — no `COUNT(*)` or slow table scans.

| Parameter         | Type   | Required | Description                                              |
| ----------------- | ------ | -------- | -------------------------------------------------------- |
| `connection_id`   | string | no       | Connection UUID, or list index `#` from `dbx_list_connections` |
| `connection_name` | string | no       | Connection name, or list index `#` from `dbx_list_connections` |
| `database`        | string | no       | Database name (Dameng: also accepted as schema alias)  |
| `schema`          | string | no       | Schema name (default: `public` for PostgreSQL, `dbo` for SQL Server) |

**Report sections:**

1. **Database Summary** — schema/db name, charset, collation, table count (from catalog)
2. **Tables** — name, type, engine, row estimate, size, table comment; sorted by rows descending
3. **Column Comments** — columns with non-empty comments from `information_schema.COLUMNS` or `pg_catalog.col_description`
4. **Indexes** — from `information_schema.STATISTICS` (MySQL) or `pg_indexes` (PostgreSQL) or `sqlite_master`

**CLI:** `dbx report <connection|#> [--schema name] [--database name] [--json]`

## SQL Safety

`dbx_execute_query` accepts multiple SQL statements and executes them one at a time after checking each statement. Regular write statements such as `INSERT`, `UPDATE`, and `DELETE ... WHERE ...` are allowed by default.

If you need to force a read-only MCP session, set:

```bash
DBX_MCP_ALLOW_WRITES=0
```

Dangerous statements such as `DROP`, `TRUNCATE`, and `ALTER` remain blocked unless you also set:

```bash
DBX_MCP_ALLOW_DANGEROUS_SQL=1
```

Redis connections use `dbx_execute_redis_command` instead of `dbx_execute_query`. Redis write commands honor `DBX_MCP_ALLOW_WRITES`; dangerous Redis commands such as `KEYS`, `FLUSHALL`, and `EVAL` require `DBX_MCP_ALLOW_DANGEROUS_SQL=1`.

## Streaming Output

Tools that connect to a database (`dbx_list_tables`, `dbx_describe_table`, `dbx_execute_query`, `dbx_get_database_stats`, `dbx_get_database_report`, `dbx_execute_redis_command`, `dbx_get_schema_context`, etc.) prepend streaming output to the tool response:

```
[dbx] Using connection "my-db" (postgres @ host:5432/mydb)
[dbx] Connecting via socks5 proxy proxy.example.com:1080
[dbx] Starting local proxy tunnel to host:5432...
[dbx] Proxy tunnel ready on 127.0.0.1:54321
[dbx] Connecting to database postgres @ host:5432/mydb (via 127.0.0.1:54321)
[dbx] Database connection pool ready

---

[my-db (uuid) [postgres @ host:5432]]
| results... |
```

On errors, the same progress lines appear before the error message so you can see which stage failed (proxy, tunnel, database, etc.).

| Variable | Default | Effect |
|----------|---------|--------|
| `DBX_MCP_QUIET=1` | off | Suppress streaming output in tool responses |
| `DBX_MCP_VERBOSE=1` | off | Include extra verbose-only steps (e.g. reused proxy tunnel, bridge endpoint) |

CLI uses the same streaming output via stderr with `--quiet` / `--verbose` or `DBX_QUIET` / `DBX_VERBOSE`.

## How It Works

```
AI Agent → MCP Server → Database
                ↓
         DBX SQLite database (dbx.db)
```

The MCP server reads your database connections from DBX's SQLite database:

- **macOS**: `~/Library/Application Support/com.dbx.app/dbx.db`
- **Linux**: `~/.local/share/com.dbx.app/dbx.db`
- **Windows**: `%APPDATA%\com.dbx.app\dbx.db`

Windows portable builds store data next to `DBX.exe`, usually in `data\dbx.db`. Set `DBX_DATA_DIR` to that `data` folder instead of copying `dbx.db` into the default directory.

## DBX Web / Docker Mode

When connecting MCP to a deployed DBX Web instance, set `DBX_WEB_URL` instead of reading local desktop storage:

```json
{
  "mcpServers": {
    "dbx": {
      "command": "dbx-mcp-server",
      "env": {
        "DBX_WEB_URL": "https://dbx.example.com",
        "DBX_WEB_PASSWORD": "your-web-password"
      }
    }
  }
}
```

If the Web instance has password protection enabled, `DBX_WEB_PASSWORD` is required. Use the same password you enter on the DBX Web login page, including the password created by the first-run setup screen. You do not need to set `DBX_PASSWORD` on the DBX Web server just for MCP; `DBX_PASSWORD` is only a server-side environment override. Without `DBX_WEB_PASSWORD`, MCP calls fail before any connection data is returned. Desktop local mode does not use `DBX_WEB_PASSWORD`.

## DBX UI Integration

The `dbx_open_table` tool communicates with the running DBX app to open tables directly in the UI. This requires DBX to be running. If DBX is not running, the tool will return an error message.

PostgreSQL, MySQL, SQLite, Doris, StarRocks, and Redshift queries run directly from the MCP server. Redis standalone command execution also runs directly. Other database types, plus Redis Sentinel/Cluster or SSH-backed Redis connections, still use the DBX desktop bridge unless `DBX_WEB_URL` is configured.

## Requirements

- [DBX](https://github.com/t8y2/dbx) installed with at least one connection configured
- Node.js 22.13.0 或更高版本

## License

Apache-2.0

---

## 中文说明

[DBX](https://github.com/t8y2/dbx) 的 MCP Server，让 AI 编程助手（Claude Code、Cursor 等）直接使用 DBX 中已配置的数据库连接查询数据。

### 特性

- **零配置** — 自动读取 DBX 的连接配置
- **9 个工具** — 列出/添加/删除连接、列出表、查看表结构、获取 Schema 上下文、执行 SQL、执行 Redis 命令、在 DBX 中打开表
- **连接池** — 跨查询复用数据库连接
- **直接执行** — PostgreSQL、MySQL、SQLite 及兼容数据库（Doris、StarRocks 等）无需打开 DBX 即可查询
- **默认允许常规写入** — `INSERT` / `UPDATE` / `DELETE` 可直接执行，危险语句仍需显式开启
- **DBX UI 联动** — 从 AI 助手直接在 DBX 桌面端打开表

### 快速开始

#### 1. 安装

```bash
npm install -g @dbx-app/mcp-server
```

或直接运行：

```bash
npx @dbx-app/mcp-server
```

#### 2. 配置 Claude Code

在项目的 `.mcp.json` 中添加：

```json
{
  "mcpServers": {
    "dbx": {
      "command": "dbx-mcp-server"
    }
  }
}
```

Windows 便携版需要在 MCP 配置中设置 `DBX_DATA_DIR`，指向包含 `dbx.db` 的便携版数据目录：

```json
{
  "mcpServers": {
    "dbx": {
      "command": "dbx-mcp-server",
      "env": {
        "DBX_DATA_DIR": "D:\\DBX_x64-portable\\data"
      }
    }
  }
}
```

#### 3. 使用

在 Claude Code 中直接说：

- "列出我的数据库连接"
- "查看 local-pg 上有哪些表"
- "查看 users 表的结构"
- "查询最近 7 天的订单数量"
- "打开 orders 表"

### CLI

终端、脚本和 Codex 工作流请安装独立 CLI 包：

```bash
npm install -g @dbx-app/cli
dbx connections list --json
dbx query local "select 1" --json
```

命令详情见 [DBX CLI README](../cli/README.md)。

### 列表序号（`#` 列）

连接、代理、表列表均包含 **`#`** 列，表示当前列表中的 **1-based 行序号**（与解析数字引用时使用的顺序一致）。

可用该序号代替 UUID 或名称：

| 输入示例 | 含义 |
| -------- | ---- |
| `1`、`#2` | 对应类别最新列表中的第 1 / 2 行 |
| 精确 UUID / 名称 | 仍优先匹配（先于序号解析） |

**MCP：** 在 `connection_id`、`connection_name`、`proxy_profile_id`、`proxy_profile_name` 中传入数字。

**CLI：** 在连接参数位置使用，例如 `dbx stats 1`、`dbx query 2 "select 1"`、`--proxy-profile-id 1`。

每次 list 都会重新加载数据；序号 `N` 表示「当前列表第 N 行」，不是持久化 ID。

### 工具列表

| 工具                        | 说明                                  |
| --------------------------- | ------------------------------------- |
| `dbx_list_connections`      | 列出 DBX 中所有已配置的数据库连接     |
| `dbx_add_connection`        | 添加新的数据库连接                    |
| `dbx_list_proxies`          | 列出 DBX 设置中已保存的代理隧道配置   |
| `dbx_remove_connection`     | 删除数据库连接                        |
| `dbx_list_tables`           | 列出指定连接的表和视图                |
| `dbx_describe_table`        | 获取表的列定义                        |
| `dbx_get_database_stats`    | 从系统目录视图获取数据库状态概览      |
| `dbx_get_database_report`   | 完整数据库报告：摘要、表、列注释、索引 |
| `dbx_get_schema_context`    | 获取适合 AI 写 SQL 的紧凑表结构上下文 |
| `dbx_execute_query`         | 执行 SQL 查询（最多返回 100 行）      |
| `dbx_execute_redis_command` | 在 Redis 连接上执行 Redis 命令        |
| `dbx_open_table`            | 在 DBX 桌面端打开指定表               |

### `dbx_add_connection` 参数

| 参数              | 类型    | 必填     | 默认值   | 说明                                     |
| ----------------- | ------- | -------- | -------- | ---------------------------------------- |
| `name`            | string  | 是       | —        | 连接名称                                 |
| `db_type`         | string  | 是       | —        | 数据库类型（如 `mysql`、`postgresql`）   |
| `host`            | string  | 是       | —        | 数据库主机；cloudflare-d1 填 Account ID  |
| `port`            | number  | 否       | —        | 数据库端口（按 db_type 有默认值）        |
| `username`        | string  | 否       | `""`     | 用户名                                   |
| `password`        | string  | 否       | `""`     | 密码；cloudflare-d1 填 API Token         |
| `database`        | string  | 否       | —        | 默认数据库；cloudflare-d1 填 D1 ID       |
| `ssl`             | boolean | 否       | `false`  | 是否启用 SSL                             |
| `driver_profile`  | string  | 否       | —        | 驱动配置（如 `gbase8a`、`gbase8s`）      |
| `proxy_enabled`   | boolean | 否       | `false`  | 是否启用 SOCKS5/HTTP 代理隧道            |
| `proxy_type`      | enum    | 否       | `socks5` | 代理协议：`socks5` 或 `http`             |
| `proxy_host`      | string  | 启用代理时 | —      | 代理服务器主机                           |
| `proxy_port`      | number  | 否       | `1080`   | 代理端口（1–65535）                      |
| `proxy_username`  | string  | 否       | —        | 代理认证用户名                           |
| `proxy_password`  | string  | 否       | —        | 代理认证密码                             |
| `proxy_profile_id` | string | 否       | —        | 已保存代理配置 ID、名称，或 `dbx_list_proxies` 中的序号 `#` |
| `proxy_profile_name` | string | 否 | —        | 已保存代理配置名称或序号 `#`（`proxy_profile_id` 的替代） |

**代理模式（互斥，不可混用）：**

1. **内联模式** — 设置 `proxy_enabled=true` 并提供 `proxy_host`（及可选的 `proxy_type`、`proxy_port`、认证信息）。通过遗留 `proxy_*` 字段写入 `transport_layers`。
2. **引用已保存配置** — 设置 `proxy_profile_id` 或 `proxy_profile_name`，引用 DBX **设置 > 隧道** 中的代理配置。连接侧只存带 `profile_id` 的 `transport_layers` 引用桩，DBX 在连接时解析完整代理配置。

不可同时使用内联代理参数与 `proxy_profile_id` / `proxy_profile_name`。引用已保存配置时只能指定 ID 或名称之一。可用 `dbx_list_proxies` 查看可用配置。

SSH 隧道参数暂未在此 MCP 工具中暴露。

### `dbx_get_database_stats`

返回适合 AI 阅读的紧凑 Markdown 概览。所有指标均来自数据库系统目录视图，不执行手动 `COUNT(*)`，也不依赖缓存计数。

| 参数              | 类型   | 必填 | 说明                                                         |
| ----------------- | ------ | ---- | ------------------------------------------------------------ |
| `connection_id`   | string | 否   | 连接 UUID，或 `dbx_list_connections` 中的序号 `#`            |
| `connection_name` | string | 否   | 连接名称，或 `dbx_list_connections` 中的序号 `#`             |
| `database`        | string | 否   | 数据库名（Dameng 也可作为 schema 别名）                      |
| `schema`          | string | 否   | Schema 名（PostgreSQL 默认 `public`，SQL Server 默认 `dbo`） |

**各数据库类型的查询策略：**

| 类型 | 目录来源 |
| ---- | -------- |
| MySQL / MariaDB / Doris / StarRocks / Manticore | `information_schema.TABLES`、`information_schema.SCHEMATA` |
| PostgreSQL 系 | `information_schema.tables` + `pg_catalog` + `pg_stat_user_tables` |
| SQLite / rqlite | `sqlite_master` |
| MongoDB | 各集合 `collStats`（最多 50 个） |
| Redis | `INFO` + `DBSIZE` |
| 其他 SQL（经 bridge） | `information_schema.tables` 尽力回退 |

输出列：Name、Type、Engine、Rows/Docs（估计值）、Data、Index、Total、Comment。

### `dbx_get_database_report`

返回结构化 Markdown 报告，适合 AI 与人工阅读。所有数据来自系统目录视图，不执行 `COUNT(*)` 或慢速全表扫描。

| 参数              | 类型   | 必填 | 说明                                                         |
| ----------------- | ------ | ---- | ------------------------------------------------------------ |
| `connection_id`   | string | 否   | 连接 UUID，或 `dbx_list_connections` 中的序号 `#`            |
| `connection_name` | string | 否   | 连接名称，或 `dbx_list_connections` 中的序号 `#`             |
| `database`        | string | 否   | 数据库名（Dameng 也可作为 schema 别名）                      |
| `schema`          | string | 否   | Schema 名（PostgreSQL 默认 `public`，SQL Server 默认 `dbo`） |

**报告章节：**

1. **Database Summary** — schema/库名、字符集、排序规则、表数量（来自目录）
2. **Tables** — 表名、类型、引擎、行数估计、大小、表注释；按行数降序排列
3. **Column Comments** — 来自 `information_schema.COLUMNS` 或 `pg_catalog.col_description` 的非空列注释
4. **Indexes** — 来自 `information_schema.STATISTICS`（MySQL）、`pg_indexes`（PostgreSQL）或 `sqlite_master`（SQLite）

**CLI：** `dbx report <connection|#> [--schema name] [--database name] [--json]`

### SQL 安全

`dbx_execute_query` 支持多条 SQL 语句，会逐条完成安全检查并依次执行。默认允许常规写操作，例如 `INSERT`、`UPDATE`、`DELETE ... WHERE ...`。

如果你希望 MCP 会话强制退回只读，可设置：

```bash
DBX_MCP_ALLOW_WRITES=0
```

`DROP`、`TRUNCATE`、`ALTER` 等危险语句仍会被拦截，除非额外设置：

```bash
DBX_MCP_ALLOW_DANGEROUS_SQL=1
```

Redis 连接使用 `dbx_execute_redis_command`，不通过 `dbx_execute_query` 执行。Redis 写命令遵循 `DBX_MCP_ALLOW_WRITES`；`KEYS`、`FLUSHALL`、`EVAL` 等危险 Redis 命令需要设置 `DBX_MCP_ALLOW_DANGEROUS_SQL=1`。

### 流逝输出

会建立数据库连接的工具（`dbx_list_tables`、`dbx_describe_table`、`dbx_execute_query`、`dbx_get_database_stats`、`dbx_get_database_report`、`dbx_execute_redis_command`、`dbx_get_schema_context` 等）会在返回结果前附带流逝输出：

```
[dbx] Using connection "my-db" (postgres @ host:5432/mydb)
[dbx] Connecting via socks5 proxy proxy.example.com:1080
[dbx] Starting local proxy tunnel to host:5432...
[dbx] Proxy tunnel ready on 127.0.0.1:54321
[dbx] Connecting to database postgres @ host:5432/mydb (via 127.0.0.1:54321)
[dbx] Database connection pool ready

---

[my-db (uuid) [postgres @ host:5432]]
| 查询结果... |
```

出错时，错误信息前同样会显示进度，便于定位失败阶段（代理、隧道、数据库等）。

| 变量 | 默认 | 作用 |
|------|------|------|
| `DBX_MCP_QUIET=1` | 关闭 | 不在工具响应中显示流逝输出 |
| `DBX_MCP_VERBOSE=1` | 关闭 | 显示更多 verbose 步骤（如复用代理隧道、bridge 端点） |

CLI 通过 stderr 输出相同流逝输出，可用 `--quiet` / `--verbose` 或 `DBX_QUIET` / `DBX_VERBOSE`。

### 工作原理

MCP Server 从 DBX 的 SQLite 数据库读取连接信息：

- **macOS**: `~/Library/Application Support/com.dbx.app/dbx.db`
- **Linux**: `~/.local/share/com.dbx.app/dbx.db`
- **Windows**: `%APPDATA%\com.dbx.app\dbx.db`

Windows 便携版的数据通常在 `DBX.exe` 同级的 `data\dbx.db`。请把 `DBX_DATA_DIR` 设置为这个 `data` 文件夹，不要手工复制 `dbx.db` 到默认目录。

### DBX Web / Docker 模式

如果 MCP 连接的是已部署的 DBX Web 实例，请设置 `DBX_WEB_URL`，不要读取本机桌面端存储：

```json
{
  "mcpServers": {
    "dbx": {
      "command": "dbx-mcp-server",
      "env": {
        "DBX_WEB_URL": "https://dbx.example.com",
        "DBX_WEB_PASSWORD": "你的 Web 访问密码"
      }
    }
  }
}
```

当 Web 实例启用了密码保护时，必须提供 `DBX_WEB_PASSWORD`。这里填写的就是 DBX Web 登录页使用的密码，也包括首次打开 Web 页面时通过 setup 设置的密码。为了让 MCP 可用，不需要在启动 DBX Web 时额外设置 `DBX_PASSWORD`；`DBX_PASSWORD` 只是服务端环境变量覆盖。未提供 `DBX_WEB_PASSWORD` 时，MCP 调用会在返回任何连接数据前失败。桌面本地模式不使用 `DBX_WEB_PASSWORD`。

### DBX UI 联动

`dbx_open_table` 工具通过本地 HTTP 接口与运行中的 DBX 应用通信，直接在 UI 中打开表。需要 DBX 正在运行。

PostgreSQL、MySQL、SQLite、Doris、StarRocks、Redshift 查询可由 MCP Server 直接执行。Redis standalone 命令执行也会直接连接。其他数据库类型，以及 Redis Sentinel/Cluster 或 SSH Redis 连接，仍会走 DBX 桌面端 bridge，除非配置了 `DBX_WEB_URL` 使用 Web 后端。

### 系统要求

- 已安装 [DBX](https://github.com/t8y2/dbx) 并配置了至少一个数据库连接
- Node.js 22.13.0 or newer
