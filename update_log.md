# Update Log

## 2026-07-22 — 修复 Web 模式 `connections update` 持久化 405

### 现象

`DBX_WEB_URL` 下执行 `dbx connections update … --proxy-profile-id …`：多代理 failover 试连可通（如 Connected via proxy #2），但写回失败：

`API request /api/connection/mcp/update failed: 405 Method Not Allowed` → `0 updated, N failed`。

### 根因

- CLI / `WebBackend::update_connection_for_mcp` 调用 **`POST /api/connection/mcp/update`**
- 已部署的 `dbx-web` 只注册了 `mcp/add`、`mcp/remove`，**没有** `mcp/update`
- 未命中 API 时落到静态资源 `ServeDir` fallback；对静态路径发 POST → **405 Method Not Allowed**（不是单纯 404）

### 改动

- `crates/dbx-web/src/main.rs`：注册 `POST /connection/mcp/update`
- `crates/dbx-web/src/routes/connection.rs`：`mcp_update_connection`（对齐 add/remove，走 `storage.update_connection_for_mcp`，并清 pool / 重置 transport）
- 配套链路（同批 WIP）：`WebBackend` → 该 API；`LocalBackend` / `Storage::update_connection_for_mcp` 本地直写

### 部署注意

修的是 **dbx-web API**。远端（如 `47.99…`）必须 **重新编译并部署 dbx-web** 后 405 才会消失。只更新本地 `dbx.exe`、远端仍是旧 API → 照样 405。

---

## 2026-07-22 — CLI `connections update` 支持 range 批量

### 问题

`dbx connections update 1-10 --proxy-profile-id 1,2,3` 若 PATH 上是旧版 `dbx`（如 npm 全局包），Usage 里看不到 `connections update`。源码侧此前已有单条 update + 多代理 failover，但 **不接受 range**（`find_connection` 对 `1-10` 直接报 CONNECTION_RANGE）。

### 改动

- `run_connections_update` 改为 `select_connections`，支持 `1-10` / `#1:#10` 等 range 批量
- 每条连接独立：多代理 failover 试连，成功则写回 winner，失败则该条保持原配置；批量有失败时 soft-fail
- range 下禁止 `--name`（一次只能改一个名字）
- Usage 更新为 `<connection|#|range>`

### 怎么跑

PATH 上的 npm `dbx` 不会带这次改动。需用仓库二进制（自行编译后）：

```powershell
& G:\rust\dbx-main-rust\target\release\dbx.exe connections update 1-10 --proxy-profile-id 1,2,3
```

---

## 2026-07-22 — CLI `connections import` 批量直写（不试连）

### 背景

大量连接配置要导入时，逐条 `connections add` / 试连太慢。`add` 本身已直写 storage、不试连；此前缺少 **JSON 批量导入** 命令。

### 行为

- 新增 `dbx connections import --file <path.json>`：从 JSON **批量写入**连接，**永不试连**
- 写入路径与 `add` 相同：`add_connection_for_mcp` → 本地 `dbx.db`；若设置了 `DBX_WEB_URL` 则经 Web API 写入远端 store
- 同名连接 **跳过**（不覆盖），其它条目继续；有失败条目时以 soft-fail（非 0 退出）并输出明细
- 支持代理：`proxy_url`（如 `socks5://user:pass@host:1080` / `http://host:8080`）或拆分字段 `proxy_host`/`proxy_port`/`proxy_type`，或 `proxy_profile_id` / `proxy_profile_name`（引用已保存隧道配置，同样不试连）

### JSON 格式

数组，或 `{ "connections": [ ... ] }`：

```json
[
  {
    "name": "pg-prod",
    "type": "postgres",
    "host": "10.0.0.1",
    "port": 5432,
    "username": "u",
    "password": "p",
    "database": "app",
    "proxy_url": "socks5://127.0.0.1:1080"
  },
  {
    "name": "mysql-edge",
    "type": "mysql",
    "host": "10.0.0.2",
    "proxy_host": "127.0.0.1",
    "proxy_port": 7890,
    "proxy_type": "http"
  }
]
```

字段别名：`type` / `dbType`、`db`、`proxyUrl`、`proxyProfileId` 等。

### 用法示例

```powershell
dbx connections import --file .\connections.json
dbx connections import --file .\connections.json -j
# 经 Web 写入（不落本地 db）
$env:DBX_WEB_URL = "http://127.0.0.1:7429"
dbx connections import --file .\connections.json -j
```

### 改动文件

- `crates/dbx-cli/src/main.rs` — `connections import` + `proxy_url` 解析 + 单测（同文件）
- `update_log.md` — 本条

### 请你本地验证

```powershell
# 准备 JSON 后
dbx connections import --file .\connections.json -j
dbx connections list -j
```

---

## 2026-07-22 — CLI `connections update` 多代理 failover 试连写回

### 行为

- 新增 `dbx connections update <connection|#>`：可改 host/port/凭证等字段，并支持多代理参数（与 `add` / MCP 约定一致）
- 指定多个代理时按顺序 **failover 试连**（不是多跳串代理）；**第一个成功的代理**写入存储，替换该连接 `transport_layers` 里原有的 proxy 部分（最终只持久化 winner，不保留整组 failover stub）
- 全部失败则报错，**不修改**原连接配置
- 仅改字段、不带代理参数时：直接更新，不强制试连

### 用法示例

```powershell
# 列表序号 1,2,3 依次试连，成功者写回
dbx connections update my-pg --proxy-profile-id 1,2,3

# UUID / #序号 / 名称（可重复 flag）；也可 --proxy-profiles
dbx connections update #2 --proxy-profile-id <uuid-a>,<uuid-b>
dbx connections update my-pg --proxy-profile-name edge-a --proxy-profile-name edge-b

# 同时改 host，并试多代理
dbx connections update my-pg --host 10.0.0.5 --proxy-profile-id "#1-#3" -j

# 内联单代理（试通后写回）
dbx connections update my-pg --proxy --proxy-type socks5 -H 127.0.0.1 --proxy-port 1080
```

### 改动文件

- `crates/dbx-cli/src/main.rs` — `connections update` + failover 选 winner 写回 + help
- `crates/dbx-core/src/storage.rs` — `update_connection_for_mcp`
- `crates/dbx-mcp/src/backend.rs` — `update_connection_for_mcp` / `test_connection_config`（Local + Web）
- `crates/dbx-web/src/routes/connection.rs` + `main.rs` — `POST /api/connection/mcp/update`
- 测试 mock：`dbx-mcp` server/protocol、`dbx-cli` MongoBackend

### 请你本地验证

```powershell
dbx proxies list
dbx connections update <name|#> --proxy-profile-id 1,2,3 -v
dbx connections list -j
```

---

## 2026-07-22 — MCP Web 模式 401：鉴权加固 + 启动诊断

### 现象

- 同一 `DBX_WEB_URL` / `DBX_WEB_PASSWORD` 下，`dbx proxies list` 成功，MCP `dbx_list_proxies` 报 401
- CLI / MCP 共用 `WebBackend`，若环境变量真一致则行为应一致 → 优先怀疑 **MCP 进程没吃到 env**，其次是鉴权状态机缺陷

### 代码修复（`crates/dbx-mcp`）

- `ensure_auth`：不再用单独的 `checked` 无 cookie 就放行（此前会导致后续 API 直接 401）
- 需要登录时：必须拿到 `dbx_session`；登录 401 / API 401 错误信息区分得更清楚，并提示检查 MCP env
- 解析全部 `Set-Cookie`（`get_all`）
- 启动时 stderr 打印：`[dbx-mcp] web mode enabled: url=... password_configured=true|false`（不打印密码）

### 排查步骤

1. 看 MCP 日志里有没有上述 stderr；`password_configured=false` = `.mcp.json` 的 `env` 没进进程
2. 改完 `.mcp.json` 后必须 **完全重启 MCP server**（不是只重载对话）
3. 用同一二进制对照：

```powershell
$env:DBX_WEB_URL="http://47.99.83.15:14224"
$env:DBX_WEB_PASSWORD="***"
dbx-mcp-server   # 看 stderr 是否 password_configured=true
```

---

## 2026-07-22 — Windows：升级 duckdb 以过 MSVC 14.51

### 原因

- Windows 本机 `cargo build --release -p dbx-web` 在编 `libduckdb-sys` 时失败（MSVC 14.51 / VS 18 Insiders）
- 已知问题：[duckdb-rs#786](https://github.com/duckdb/duckdb-rs/issues/786)，`1.10503.1` 在该工具链上编 bundled DuckDB 会挂
- 上游已在 **v1.10504.0** 修复

### 改动

- `crates/dbx-core/Cargo.toml`、`src-tauri/Cargo.toml`：`duckdb` `1.3.2` → `1.10504.0`
- `Cargo.lock`：`duckdb` / `libduckdb-sys` → `1.10504.0`

### 请你本地验证

```powershell
# 建议先清掉坏掉的 duckdb 构建缓存
Remove-Item -Recurse -Force target\release\build\libduckdb-sys-* -ErrorAction SilentlyContinue
$env:CARGO_BUILD_JOBS = "2"
cargo build --release -p dbx-web
```

> bundled DuckDB 很吃内存；若仍偶发 `cl.exe` exit 2，可再降到 `CARGO_BUILD_JOBS=1`。

---

## 2026-07-22 — 整仓检出 + 自有 Docker/WSLC 镜像配置

### 仓库

- 原为 **sparse checkout**（仅 `crates` / 部分 `packages` / `src-tauri`），已 `git sparse-checkout disable` 拉全树（含 `deploy/`、`apps/`、`docs/` 等）
- `git fetch --unshallow` 完成；当前相对 `origin/main`：本地 ahead 2 / behind 75（含本地 mcp patches 提交）

### 自有镜像（用 **WSLC**，不用 Docker Desktop）

- 官方 Dockerfile：[`deploy/Dockerfile`](deploy/Dockerfile)（GHA / Docker Buildx + zig 交叉编译）
- 自有 WSLC 构建：[`deploy/Dockerfile.self`](deploy/Dockerfile.self)（原生 amd64、无 BuildKit cache/`--platform`）
- Compose：[`deploy/docker-compose.self.yml`](deploy/docker-compose.self.yml)，镜像名 **`dbx-self:latest`**

```powershell
cd G:\rust\dbx-main-rust
wslc build -f deploy/Dockerfile.self -t dbx-self:latest .
# 或
wslc-compose -f deploy/docker-compose.self.yml build
wslc-compose -f deploy/docker-compose.self.yml up -d
# http://localhost:4224  默认密码 changeme
```

- 构建日志：`wslc-build.log` / `wslc-build.log.err`（本地）

---

## 2026-07-22 — Web API 对齐 MCP/CLI 后加能力（stats/report / timeout / proxy）

### 共享逻辑下沉 `dbx-core`

- 新增 [`crates/dbx-core/src/database_stats.rs`](crates/dbx-core/src/database_stats.rs)、[`database_report.rs`](crates/dbx-core/src/database_report.rs)
- `CatalogStatsExecutor` trait：`execute_query` / `execute_redis_command` / `list_tables` / `mongo_collection_stats`
- **Mongo stats** 改为 `mongo_collection_stats`，不再走 `execute_query("db.x.stats()")`
- `list_index`、`proxy_profiles`（原 MCP `tunnel_profiles`）迁入 core；MCP 侧改为 `pub use` 再导出

### Web 新路由

| 方法 | 路径 | 说明 |
|------|------|------|
| POST | `/api/database/stats` | 目录估算 stats（markdown） |
| POST | `/api/database/report` | 完整 report |

请求体（camelCase）：

- `connectionId` 或 `connectionIds`（批量）
- `database?` / `schema?` / `redisDb?`
- `timeoutMs?`
- `skipUnsupported?`（默认 `true`）
- `proxyProfileId?` / `proxyProfileName?` / `proxyProfileIds?` / `proxyProfileNames?`

响应：`{ markdown, total, success, skipped, failures }`

Proxies 列表仍用已有 `GET /api/tunnel-profiles/list`（等价 `dbx_list_proxies`）。

### WebBackend 修复

- `/api/query/execute` 转发 `timeoutSecs`、`maxRows`
- Web 模式 stats/report **优先**调用 `/api/database/stats|report`（一次往返）

### 本地验证（请自行编译）

```bash
cargo build -p dbx-core --no-default-features
cargo build -p dbx-mcp --release
cargo build -p dbx-cli --release --no-default-features
cargo build -p dbx-web --no-default-features
```
