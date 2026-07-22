# Update Log

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
