# Update Log

## 2026-07-24 — 技能：connection-url（自包含明文列表）

### 背景

用户要求把 `connectionUrl.test.ts` 里的契约**直接写进技能**，运行时不再去读测试/源码；输出明文字段列表，不脱敏。

### 交付

- [`skills/connection-url/SKILL.md`](skills/connection-url/SKILL.md) — scheme 表 + 全套输入→结果样例（自包含）
- 已删 `schemes.md`（避免再跳转读文件）

---

## 2026-07-23 — 本地脚本：结构克隆 + 样例 SQL（不改 report 产品）

### 背景

用户要「全表结构 + 少量最新样例」，但明确：**不要**改成 `dbx report` / MCP report 产品能力；写成**本机脚本**，用现有 `dbx-cli` 生成可复制的 **SQL**（DDL + 样例 SELECT/INSERT）。

### 交付

- 唯一脚本：[`scripts/schema-clone-sample.ps1`](scripts/schema-clone-sample.ps1)
- **未改** `database_report` / CLI report / MCP report

### 行为

1. `dbx-cli schema list` 列表面
2. 每表 `schema describe -j` → 拼 `CREATE TABLE IF NOT EXISTS`（字段名/类型/可空/默认/注释 + PRIMARY KEY）
3. 有数据的基表：`ORDER BY` 单列 PK，否则时间列（`updated_at`/`create_time`/…），再否则无序；`LIMIT` 默认 **20**
4. 写出样例 `SELECT`（注释）+ `INSERT` 行到 `schema-clone.sql`

### 用法

```powershell
# 默认输出到 .\schema-clone-<连接名>-<时间戳>\schema-clone.sql
.\scripts\schema-clone-sample.ps1 -Connection whatsapp_call

# 指定库 / 样例行数 / 输出目录
.\scripts\schema-clone-sample.ps1 -Connection 12 -Database mydb -SampleRows 10 -OutDir .\out\clone

# 仅 stdout
.\scripts\schema-clone-sample.ps1 -Connection whatsapp_call -Stdout

# 指定 dbx-cli
.\scripts\schema-clone-sample.ps1 -Connection myconn -DbxCli .\target\release\dbx-cli.exe
```

依赖：已有 `target\release\dbx-cli.exe`（或 PATH / `-DbxCli`）。脚本**不** cargo build。

### 倒序选列

1. 唯一主键列；复合主键取第一个 PK
2. 否则按名匹配：`updated_at` / `update_time` / `created_at` / `create_time` / `id` 等
3. 否则任意 `date|time|timestamp` 类型列
4. 都没有 → `SELECT * … LIMIT n`（无 ORDER BY）

---

## 2026-07-23 — 代理 failover 日志去掉误导性 `→`

### 背景

跑 `query`/`stats` 时日志出现：
`Proxy failover group ...: #1 (21087-2) → #2 (21087-3)`，用户误以为代理被串联（multi-hop）。

### 核实结论

1. **日志出处**：`crates/dbx-mcp/src/progress.rs` 的 `log_using_connection`（非 `tunnel_profiles`）。`→` 仅是 `labels.join(" → ")` 的展示分隔符。
2. **运行时不是串联**：`connection.rs` 的 `transport_layer_failover_attempts` 识别「首个 Proxy `enabled=true`、后续 `enabled=false`」为 failover 组；每次 attempt 只塞 **一个** Proxy + 非 Proxy 层。`start_transport_layers` 在单次 attempt 里才会链式连接多层。
3. **import stubs 路径正确**：`apply_proxy_profiles_failover` 存的就是上述 stub 约定；`Using connection` 日志与 runtime 都按 failover 处理，不会把禁用 stub 当成链式 hop。

### 改动

- 日志改为：`Proxy failover candidates: #1 (A), #2 (B) (try next on failure, not multi-hop chained)`，去掉 `→`。

### 涉及文件

- `crates/dbx-mcp/src/progress.rs`
- `update_log.md`

---

## 2026-07-23 — import 支持 jdbc: / 自动 name / 无表头 URL 列表

### 背景

用户批量导入 JDBC URL（`jdbc:postgresql://` / `jdbc:mysql://`）时报 `name is required`；纯文本「每行一个 URL」无 CSV 表头也无法导入。密码中的 `@` 需提醒编码。

### 改动

1. **`jdbc:` 前缀**：`connection_url.rs` 解析前 strip `jdbc:`，与 `mysql` / `postgresql` / `mariadb` 等 scheme 对齐；补充单测与 scheme 列表（`jdbc:mysql` / `jdbc:postgresql` / `jdbc:mariadb`）。
2. **缺 name 自动生成**：import 时若无 `name` 列且 URL 无 `name=`，从 `host` 或 `host/database` 生成；与已有连接/同批冲突时加 `-2`、`-3`… 后缀。
3. **无表头 URL 列表**：`.txt` / `.csv` 若每行都是连接 URL（可含 `#` 注释行），按「一行一个 url」导入，无需表头。
4. **密码含 `@`**：authority 取最后一个 `@` 分隔 userinfo/host（常见写法可用）；文档与错误提示仍要求将密码中的 `@` 编为 `%40`，避免跨工具歧义。不静默猜错。

### 用法提示

- CSV **推荐带 `name` 列**（可读性好）；不带也能导入。
- 已支持 `jdbc:mysql://` / `jdbc:postgresql://` / `jdbc:mariadb://` 等。
- 密码特殊字符请 URL 编码（`@` → `%40`）。

```text
jdbc:postgresql://u:p@10.0.0.1:5432/app
jdbc:mysql://root:s@10.0.0.2:3306/db
```

```powershell
dbx-cli connections import --file urls.txt
dbx-cli connections import --file connections.csv
```

### 涉及文件

- `crates/dbx-core/src/connection_url.rs`
- `crates/dbx-cli/src/main.rs`
- `update_log.md`

---

## 2026-07-23 — connections import 主推 CSV（JSON 兼容）

### 背景

批量导入连接时，JSON 对人类编辑不够友好。改为 **CSV 主推**（JSON 仍兼容）；写库逻辑不变，**不试连、同名跳过**。

### 格式识别

1. `--format csv|json` 强制指定输入格式（`-j/--json` 仍只控制**输出**）
2. 否则按扩展名：`.csv` → CSV，`.json` → JSON
3. 否则按内容：以 `[` / `{` 开头 → JSON，其余 → CSV

### CSV 表头（英文）

| 列 | 必填 | 说明 |
| --- | --- | --- |
| `name` | 可选（推荐） | 连接显示名；缺省时从 host[/database] 自动生成；也可写在 URL 的 `name=` 参数里 |
| `url` | 推荐 | 数据库连接 URL（如 `mysql://u:p@h:3306/db` 或 `jdbc:postgresql://...`）；别名 `connection_url` / `dsn` |
| `type` | 无 url 时 | 数据库类型；也可写 `db_type` |
| `host` / `port` / `username` / `password` / `database` / `ssl` / `driver_profile` | 无 url 时 | 拆分字段；可与 url 并存（显式列覆盖 URL） |
| `proxy_profile_id` | 可选 | 已保存代理，支持 `1` 或 `1,2,3` |
| `proxy_url` | 可选 | 单条内联代理，如 `socks5://127.0.0.1:1080`（勿与 profile 混用） |

含逗号/引号的字段请用 RFC4180 双引号包裹（内部 `"` 写成 `""`）。

### CSV 示例

```csv
name,url,proxy_profile_id
prod-mysql,mysql://root:secret@10.0.0.1:3306/app,1
pg-warehouse,postgres://u:p@db.example:5432/warehouse?sslmode=require,
"note,prod","mysql://root:a,b@10.0.0.1:3306/app","1,2"
```

第三行演示：名称/URL/代理 ID 含逗号时用双引号（RFC4180）。拆分列写法：

```csv
name,type,host,port,username,password,database,ssl
legacy,mysql,10.0.0.2,3307,u,p,app,true
```

```powershell
dbx-cli connections import --file connections.csv
dbx-cli connections import --file connections.txt --format csv -j
dbx-cli connections import --file connections.json   # 仍可用
```

### 涉及文件

- `crates/dbx-cli/src/main.rs`
- `crates/dbx-cli/Cargo.toml`（`csv`）
- `update_log.md`

---

## 2026-07-23 — connections import / add 支持数据库连接 URL

### 背景

`connections import` 原先要求 JSON 显式写 `type` / `host` / `port` 等字段。桌面端已有 `parseConnectionUrl`（`apps/desktop/src/lib/connection/connectionUrl.ts`），Rust 侧此前只有「拼 URL」、没有「解析 URL」。本次在 `dbx-core` 增加等价解析，供 CLI 批量导入与 `connections add` 使用。

**注意：这是数据库连接 URL / DSN，不是 `proxy_url`（SOCKS/HTTP 代理）。二者可并存。**

### 改动

- 新增 `crates/dbx-core/src/connection_url.rs`：`parse_connection_url` / `parse_connection_url_with_profile`
- `dbx-cli connections import`：JSON 条目支持 `url` / `connection_url` / `connectionUrl` / `dsn`
- 解析出 `type` / `host` / `port` / `username` / `password` / `database` / `ssl` / `driver_profile` 等；显式字段可覆盖 URL
- 仍可同时写 `proxy_profile_id` / `proxy_url`（代理）；批量导入**不试连、直写**
- `connections add` 增加 `--url` / `--connection-url` / `--dsn`（有半成品时可用 URL 代替 `--type`/`--host` 等）
- usage 已更新；单测覆盖 URL 导入与 flag 别名

### 支持的 URL scheme（与桌面端一致）

| Scheme | 映射类型 | 默认端口 |
| --- | --- | --- |
| `mysql` / `mariadb` | mysql | 3306 |
| `postgres` / `postgresql` | postgres | 5432 |
| `redshift` | redshift | 5439 |
| `redis` / `rediss` | redis | 6379 |
| `mongodb` / `mongodb+srv` | mongodb | 27017 |
| `sqlserver` / `mssql` / `jdbc:sqlserver://…` | sqlserver | 1433 |
| `oracle` / `jdbc:oracle:thin:@…` | oracle | 1521 |
| `clickhouse` / `elasticsearch` / `qdrant` / `milvus` / `weaviate` / `chromadb` | 同名 | 各默认 |
| `dm` / `dameng` | dameng | 5236 |
| `kingbase` / `kingbase8` | kingbase | 54321 |
| `gaussdb` / `opengauss` | gaussdb | 5432 |
| `kwdb` / `questdb` / `tdengine` / `taos-ws` / `etcd` / `zookeeper` / `gbase` / `yashandb` / `oscar` / `xugu` / `iotdb` / `iris` / `informix-sqli` / `gbasedbt-sqli` | 见解析表 | 各默认 |
| `http` / `https` | 需配合 `driver_profile`（clickhouse / elasticsearch 等） | — |

**不支持（无通用 URL 惯例，请用显式 JSON 字段）：** SQLite / DuckDB 文件路径、BigQuery、Snowflake、Hive/Spark/Trino 专用 JDBC 形态、MQ、Access/H2 等桌面端另有专用解析但 CLI 本次未全量移植的类型。

### JSON 示例

```json
[
  {
    "name": "prod-mysql",
    "url": "mysql://root:secret@10.0.0.1:3306/app?ssl-mode=required",
    "proxy_url": "socks5://127.0.0.1:1080"
  },
  {
    "dsn": "postgres://u:p@db.example:5432/warehouse?sslmode=require&name=pg-warehouse",
    "proxy_profile_id": "1"
  },
  {
    "name": "legacy",
    "type": "mysql",
    "host": "10.0.0.2",
    "port": 3307,
    "username": "u",
    "password": "p",
    "database": "app"
  }
]
```

```powershell
dbx-cli connections import --file connections.json --json

# 单条添加（URL 填 type/host/port/user/pass/db）
dbx-cli connections add --name mydb --url "mysql://user:pass@host:3306/dbname"
```

### 涉及文件

- `crates/dbx-core/src/connection_url.rs`（新）
- `crates/dbx-core/src/lib.rs`
- `crates/dbx-cli/src/main.rs`
- `update_log.md`

### 说明

- **本次未编译**（按用户要求）

---

## 2026-07-23 — 多代理 failover：数据库鉴权失败立即停止

### 问题

`connections update` 多代理 failover 时，若数据库账号/密码错误（如 MySQL 1045、Postgres `password authentication failed`），仍会继续试后续代理，浪费时间且误导排查。

### 改动

- 在 `apply_proxy_failover_pick_winner` 中：SOCKS/代理连不上/超时仍试下一个；**数据库鉴权失败立即停止**
- 新增 `is_auth_failure`（大小写不敏感）：匹配 `1045`、`Access denied`、`auth user failed`、`password authentication failed` 等；**排除** SOCKS/SSH 传输层鉴权错
- 错误码：`AUTH_FAILED`；**不修改**原连接，不试剩余代理
- 单测：`detects_db_auth_failures_for_proxy_failover_stop`

### 示例错误输出

```text
[dbx] Trying proxy #1 (office-socks)...
[dbx] Proxy #1 failed: ERROR 1045 (28000): Access denied for user 'root'@'...' (using password: YES)
Error [AUTH_FAILED]: Database authentication failed via proxy #1 (office-socks); not trying remaining proxies. Original connection was not modified. ERROR 1045 ...
```

```json
{
  "error": {
    "code": "AUTH_FAILED",
    "message": "Database authentication failed via proxy #1 (...); not trying remaining proxies. Original connection was not modified. ..."
  }
}
```

### 涉及文件

- `crates/dbx-cli/src/main.rs`
- `update_log.md`

### 说明

- **本次未编译**（按用户要求）

---

## 2026-07-23 — `connections update` 支持连接级并行（`-P`）

### 哪个工程

代理「试连 + 写回 winner」在 **`crates/dbx-cli`** 的 `connections update`（依赖 `dbx-mcp` / `dbx-core` 的 proxy failover）。不是桌面、不是单独新工程。

### 改动

- `connections update` 对齐 `stats` / `report` / `query`：支持 **`-P` / `--parallel [n]`**（默认并发 15）
- 并行粒度 = **多条 connection**；单条连接内多代理仍 **按序 failover**（试 1 失败再 2），避免同连接竞态写库
- 未传 `-P` 时仍串行（行为与改前一致）
- usage 已补充说明

### 用法

```powershell
# 对连接 1-10 并行试代理 1、2，每条连接内仍按序 failover，写回 winner
dbx-cli connections update 1-10 --proxy-profile-id 1,2 -P 5

# 默认并发 15（省略 n）
dbx-cli connections update 1-10 --proxy-profile-id 1,2 -P
```

### 涉及文件

- `crates/dbx-cli/src/main.rs`
- `update_log.md`

### 说明

- **本次未编译**（按用户要求）

---

## 2026-07-22 — 补全 `ProxyTunnelConfig::test_target` 初始化遗漏

- `ProxyTunnelConfig` 新增字段 `test_target: Option<String>`（默认 `None`）后，`tunnel_profiles.rs` / `proxy_profiles.rs` 构造处未赋值导致 E0063。
- 已在上述两处（含测试）统一补上 `test_target: None`。

## 2026-07-22 — CLI 二进制改名为 `dbx-cli`（避免与桌面主程序同名覆盖）

### 问题

桌面主程序（`src-tauri`，package `dbx`）与 CLI（`crates/dbx-cli`，原 `[[bin]] name = "dbx"`）都产出 `target/.../dbx` / `dbx.exe`，后编译者覆盖前者。

### 改动

- CLI 可执行文件改为 **`dbx-cli`**（Windows：`dbx-cli.exe`）
- 桌面主程序仍为 **`dbx` / `dbx.exe`**（未改）
- MCP 本就是 `dbx-mcp`，无冲突
- 帮助 / usage / npm launcher / 平台包 bin 路径 / 发布工作流 / skill / 文档已对齐为 `dbx-cli`

### 用法

```powershell
# 本地 cargo 产出后
& G:\rust\dbx-main-rust\target\release\dbx-cli.exe connections update 1-10 --proxy-profile-id 1,2,3

# npm 全局安装后（命令名亦为 dbx-cli）
dbx-cli connections list --json
```

### 涉及文件

- `crates/dbx-cli/Cargo.toml`、`src/main.rs`
- `packages/cli/bin/dbx.js`、`package.json`、`README.md`
- `packages/cli-*/package.json`
- `.github/workflows/mcp-release.yml`
- `skills/dbx/SKILL.md`、`README.md`、`crates/README.md`

### 说明

- **本次未编译**（按用户要求）
- 仅改 monorepo；若有独立 patches 仓依赖旧 `dbx` CLI 二进制名，需自行同步为 `dbx-cli`

---

## 2026-07-22 — 桌面 Remote API：本地 UI 连接远程 dbx-web

### 目标

服务器只跑 `dbx-web` API；本机跑桌面 UI / CLI / MCP，通过远程 API 基址访问连接存储与执行能力（类似「AI API」连法）。

### 用法（桌面）

设置 → **Remote API / 远程 API**：

1. **API 基址**：例如 `https://dbx.example.com` 或带上下文路径 `https://dbx.example.com/tools/dbx`
2. **Web 登录密码**：与远程 Web 登录密码相同
3. 点「保存并重载」；清空 URL 或点「改回本地后端」恢复 Tauri 本地后端

配置持久化在 localStorage：`DBX_WEB_URL` / `DBX_WEB_PASSWORD`（键名与 CLI/MCP 环境变量对齐）。

### 与 CLI / MCP 对应

| 客户端 | 配置 |
|--------|------|
| 桌面 Settings | `DBX_WEB_URL` + `DBX_WEB_PASSWORD`（localStorage） |
| 桌面调试覆盖 | `VITE_DBX_WEB_URL` / `VITE_DBX_WEB_PASSWORD`（构建/启动时注入，优先生效） |
| CLI / MCP | 环境变量 `DBX_WEB_URL` + `DBX_WEB_PASSWORD` |

### 鉴权 / CORS

- 登录 `POST /api/auth/login`，响应 JSON 增加 `session`；远程客户端用请求头 `X-DBX-Session`（SSE/下载/WebSocket 用查询参数 `dbx_session`，因无法设自定义头）
- Cookie `dbx_session` 仍用于同源 Web UI；MCP/CLI 继续用 Cookie
- `dbx-web` 增加宽松 CORS（`Allow-Origin: *`，允许 `X-DBX-Session`），便于 `tauri://` / 本地 UI 跨域访问远程 API

### 部署注意

- 服务器可只跑 API（不设 `DBX_STATIC_DIR` 即不托管前端静态资源）
- 若前面有反向代理，基址需包含 `DBX_PUBLIC_BASE_PATH` 对应前缀
- 跨域依赖上述 CORS；勿再强制浏览器 Cookie 跨站（已改用 session header）

### 主要改动文件

- `apps/desktop/src/lib/backend/remoteApiConfig.ts` / `remoteApiAuth.ts`（新建）
- `apps/desktop/src/lib/backend/api.ts`、`http.ts`、`webPath.ts`
- `apps/desktop/src/components/editor/EditorSettingsDialog.vue` + i18n
- `crates/dbx-web/src/auth.rs`、`main.rs`（CORS）

---

## 2026-07-22 — 修复 WSLC `Dockerfile.self` context 扫到 node_modules symlink

### 现象

```
wslc build -f deploy/Dockerfile.self -t dbx-self:latest .
```

失败：`readlink …/packages/mongo-shell/node_modules/typescript: operation not permitted`

### 根因

build context 为仓库根 `.` 时，sender 会遍历宿主上的 `node_modules`（含 Windows/WSL 挂载下不可 readlink 的 symlink）。`-f deploy/Dockerfile.self` 默认吃的是**根目录** `.dockerignore`；原先只有 `deploy/Dockerfile.self.dockerignore`，根目录没有，嵌套 `node_modules` 未被排除。

### 改动

- 新增根目录 [`.dockerignore`](.dockerignore)：排除 `**/node_modules`、`target`、`.git`、`*.exe`、`.omc`、日志等
- 同步加强 [`deploy/Dockerfile.self.dockerignore`](deploy/Dockerfile.self.dockerignore)
- [`deploy/Dockerfile.self`](deploy/Dockerfile.self) 注释写明从仓库根构建；frontend 仍 `COPY packages/mongo-shell` 后靠镜像内 `pnpm install` 装依赖（不依赖宿主 node_modules）

### 正确构建命令

在仓库根执行：

```powershell
wslc build -f deploy/Dockerfile.self -t dbx-self:latest .
```

或：

```powershell
wslc-compose -f deploy/docker-compose.self.yml build
```

---

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
