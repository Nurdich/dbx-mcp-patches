# Update Log

## 2026-07-17 — CLI 参数短别名

常用长选项增加单字母短别名，`dbx help` 与 README 统一为 `-x, --long` 格式。

### 别名表

| 短 | 长 | 说明 | 冲突处理 |
|----|-----|------|----------|
| `-j` | `--json` | JSON 输出 | 新增 |
| `-q` | `--quiet` | 关闭 stderr 进度 | 已有 |
| `-v` | `--verbose` | 额外细节 | 已有 |
| `-P [n]` | `--parallel [n]` | 并行批量 | 已有 |
| `-d NAME` | `--database` | 目标库 | 新增 |
| `-s NAME` | `--schema` | 目标 schema | 新增 |
| `-t DUR` | `--timeout` | 查询超时 | 新增 |
| `-H HOST` | `--proxy-host` | 代理主机（connections add） | `-h` 已用于 help |
| `-o PATH` | `--output` | 报告输出路径（report） | 已有（report save） |
| `-n` | `--no-save` | 跳过报告保存 | 新增 |
| `-h` | `--help` | 帮助 | 已有 |
| `-V` | `--version` | 版本 | 已有 |

**仍为长选项-only：** `--file`、`--limit`、`--format`、`--allow-writes`、`--proxy`、`--name`、`--type`、`--host`、`--port` 等（避免与 `-p`/`-f`/`-t` 等冲突或语义不清）。

### 修改文件

- `packages/cli/src/cli.ts` — `parseFlags`、`usage()`、错误路径 `-j` 识别
- `packages/cli/README.md`
- `packages/cli/dist/cli.js`（同步）

---

## 2026-07-17 — `dbx report` 默认保存到文件

### 变更摘要

`dbx report` 现在**默认将报告写入文件**，同时仍完整输出到 stdout；保存路径提示输出到 stderr（`[dbx] Report saved: ...`）。

### 默认路径

| 模式 | 路径 |
|------|------|
| 单连接 | `{DBX app data}/reports/dbx-report-{connection}-{database\|schema}-{YYYYMMDD-HHMMSS}.md` |
| 批量（如 `23-50`） | `{DBX app data}/reports/dbx-report-batch-{timestamp}/dbx-report-{connection}-{scope}.md`（每个成功连接一个文件） |
| `--json` | 同上，扩展名 `.json` |

`DBX app data` = `dbx doctor` 中的 App data directory（可用 `DBX_DATA_DIR` 覆盖）。

### 新增 CLI 标志

| 标志 | 说明 |
|------|------|
| `--no-save` | 跳过文件写入（仅 stdout） |
| `--output` / `-o` | 单连接：指定输出文件；批量：指定输出目录 |

### 修改文件

- `packages/node-core/src/database-report.ts` — 路径/文件名 helper
- `packages/cli/src/cli.ts` — 默认保存、`--no-save`、`-o`
- `packages/cli/README.md`
- `packages/mcp-server/src/index.ts` — 工具描述注明 CLI 默认保存

### 已安装 dist（手动同步）

- `packages/node-core/dist/database-report.js`、`.d.ts`
- `packages/cli/dist/cli.js`、`.d.ts`
- `G:\usr\local\node_modules\@dbx-app\cli\dist\`

---

## 2026-07-17 — 批量并行：遇错继续 + 修复连接池/日志串线

### 问题 1：批量遇错即停

`dbx stats 23-50 -P 3` 中任一连接失败（如密码错误、Access denied），`Promise.all` + 未捕获异常导致**整批中止**，后续连接不再执行。

### 问题 2：并行日志/连接池串线（#30 显示连到 realauth）

终端现象：
```
Resolved connection "..._riskmanage_c" ... kfpt_riskmanage from ref "#30"
Connecting to database mysql @ ... kfpt_realauth (via ...)
Access denied for user 'u_kfpt_realauth'@'%' to database 'kfpt_realauth'
```

**根因（双重）：**

1. **连接日志全局 stack 竞态**：`runWithConnectionLog` 用共享 `stack[]`，并行 worker 在 `await connectionEndpoint()` 时互相 push/pop，导致 `connectionLog()` 读到错误 worker 的 sink/上下文；resolve 阶段无 `[#N]` 前缀，更易与并行 Connecting 日志混淆。
2. **连接池 key 过窄 + 创建竞态**：`poolKey` 仅 `id:database`，未含 username/host/port；并发 `getMysqlPool` 无 inflight 锁，存在 TOCTOU 重复创建/覆盖风险。

`executeQuery` 本身始终使用传入的 `config` 参数，无 stale global config；问题在池缓存 key 与并行日志隔离。

### 修复

| 项 | 变更 |
|----|------|
| `runConnectionBatch` | 每连接 try/catch，失败写 stderr（`[#N] name: msg`），**继续下一连接** |
| 批量输出 | 失败连接正文显示 `**Error** (...)`；末尾 `Batch: X/Y succeeded` + Failures 列表 |
| 退出码 | 任一失败 exit 1，但 stdout 仍含全部成功+失败结果 |
| `connection-log.ts` | `AsyncLocalStorage` 隔离并行 worker 日志上下文 |
| `poolKey` | `id:db_type:host:port:username:database` |
| `getPgPool` / `getMysqlPool` | `poolInflight` 去重，并发同 key 共享创建 Promise |
| 批量 worker | 传入 `{ ...config }` 浅拷贝，避免共享对象被改写 |

### 修改文件

- `packages/cli/src/cli.ts` + `dist/cli.js`
- `packages/node-core/src/connection-log.ts` + `dist/connection-log.js`
- `packages/node-core/src/database.ts` + `dist/database.js`

---

## 2026-07-17 — 修复 stats 未使用连接默认 database 导致全库扫描超时

### 根因

`resolveCatalogStatsScope()` 仅读取 CLI/MCP 传入的 `--database` / `database` 参数，**忽略** `ConnectionConfig.database`。当连接已配置 `database: "kfpt_robot_resource"` 但用户未传 `--database` 时：

1. `buildCatalogStatsSql()` 生成 `TABLE_SCHEMA NOT IN ('information_schema', ...)` — 扫描**整台服务器所有用户库**
2. 输出标签显示 `(all user scopes)`
3. 大实例上 `information_schema.TABLES` 全库扫描超过 30s 超时

`metadataScope()` 虽将 database 写入连接 config，但 SQL 作用域未继承该值。

### 修复

| 项 | 变更 |
|----|------|
| `resolveCatalogStatsScope()` | 导出；MySQL/Dameng 默认 `options.database \|\| config.database` |
| `buildCatalogStatsSql()` | 单库模式加 `TABLE_SCHEMA = 'xxx'`；仅查 `BASE TABLE`（跳过 VIEW） |
| `fetchDatabaseStats()` | 移除 `buildCatalogSummarySql` 二次查询，仅一次主查询 + JS 摘要推导 |
| `deriveCatalogSummaryFromStats()` | 单库模式输出 `database_name` + `table_count` |
| `database-report.ts` | 复用导出的 `resolveCatalogStatsScope`，删除重复实现 |

### 修复后 SQL 示例（连接 #23，`database=kfpt_robot_resource`，无 `--database`）

```sql
SELECT TABLE_NAME AS name, TABLE_TYPE AS type, ENGINE AS engine,
       TABLE_ROWS AS rows_estimate, DATA_LENGTH AS data_bytes,
       INDEX_LENGTH AS index_bytes,
       (COALESCE(DATA_LENGTH, 0) + COALESCE(INDEX_LENGTH, 0)) AS total_bytes,
       TABLE_COMMENT AS comment
FROM information_schema.TABLES
WHERE TABLE_SCHEMA = 'kfpt_robot_resource'
  AND TABLE_TYPE = 'BASE TABLE'
```

### 批量 `dbx stats 23-50`

每个连接独立解析自身 `config.database`，不再共用全库扫描模式。

### 修改文件

- `packages/node-core/src/database-stats.ts`
- `packages/node-core/src/database-report.ts`
- `packages/node-core/dist/database-stats.js` / `.d.ts`
- `packages/node-core/dist/database-report.js`

---

## 2026-07-17 — stats 摘要移除 SQL COUNT，改 JS 内存推导

### 变更摘要

`dbx stats` / `dbx report` 的 Database Summary 不再执行任何 `COUNT(*)` / `COUNT(DISTINCT ...)` 聚合 SQL；汇总计数改自主查询 `TABLES` / `sqlite_master` 结果在 JS 中推导。

### 移除的 COUNT 查询（3 条）

| 场景 | 原 SQL | 现方案 |
|------|--------|--------|
| MySQL 全库模式 | `COUNT(DISTINCT TABLE_SCHEMA)`, `COUNT(*)` on `information_schema.TABLES` | 主查询行数 → `table_count`；唯一 `database_name` → `database_count` |
| PostgreSQL 全 schema 模式 | `COUNT(DISTINCT table_schema)`, `COUNT(*)` on `information_schema.tables` | 唯一 `schema_name` → `schema_count`；行数 → `table_count`；`database_name` 取自连接配置 |
| SQLite | `COUNT(*)` on `sqlite_master` | 主查询行数 → `object_count`；`database_name` 固定 `main` |

### 保留的非 COUNT 摘要 SQL

- MySQL 指定库：`information_schema.SCHEMATA`（charset/collation）
- PostgreSQL 指定 schema：`pg_database_size` + `current_database()`

### 未改动

- 主表目录查询仍用 `TABLE_ROWS` / `n_live_tup` / `pg_relation_size` 等目录元数据
- MongoDB `collStats` 返回的 `count` 字段（服务端元数据，非 SQL COUNT）

### 新增导出

- `deriveCatalogSummaryFromStats()` — 从主查询 `QueryResult` 生成摘要文本

### 变更文件

| 文件 | 变更 |
|------|------|
| `packages/node-core/src/database-stats.ts` | 移除 COUNT 摘要 SQL；新增 JS 推导 |
| `packages/node-core/src/database-report.ts` | 复用 `deriveCatalogSummaryFromStats` |
| `packages/node-core/dist/database-stats.js` / `.d.ts` | 同步 |
| `packages/node-core/dist/database-report.js` | 同步 |
| `C:\usr\local\node_modules\@dbx-app\mcp-server\node_modules\@dbx-app\node-core\dist\` | 同步 |
| `G:\usr\local\node_modules\@dbx-app\cli\node_modules\@dbx-app\node-core\dist\` | 同步 |

---

## 2026-07-17 — 范围无跨度上限；15 = 默认并行并发数

### 语义变更

原先 `MAX_LIST_INDEX_RANGE_SIZE = 15` 限制单次范围跨度（如 `23-50` 报 28 > 15）。**15 现指 `--parallel` 默认并发数**，不再限制范围跨度。

| 场景 | 行为 |
|------|------|
| `dbx stats 23-50` | 允许，28 个连接，**顺序**执行 |
| `dbx stats 23-50 --parallel` | 28 个连接，**最多 15 并发**（默认） |
| `dbx stats 23-50 -P 3` | 28 个连接，**最多 3 并发** |
| 范围 > 100 | 允许，stderr 输出警告（不阻断） |

### 变更文件

| 文件 | 变更 |
|------|------|
| `packages/node-core/src/list-index.ts` | 删除 `MAX_LIST_INDEX_RANGE_SIZE` 跨度校验；新增 `DEFAULT_PARALLEL_CONCURRENCY=15`、`MAX_LIST_INDEX_RANGE_WARN_SIZE=100` |
| `packages/cli/src/cli.ts` | 从 node-core 导入默认并发；大范围警告；usage 文案 |
| `packages/cli/README.md` | 文档更新 |
| dist 同步 | `packages/node-core/dist`、`packages/cli/dist`、`G:\usr\local\node_modules\@dbx-app\cli`、mcp-server node-core |

### 示例

```bash
dbx stats 23-50              # 顺序，28 连接
dbx stats 23-50 --parallel   # 并行，默认 15 并发
dbx stats 23-50 -P 3         # 并行，3 并发
```

---

## 2026-07-17 — CLI 批量范围并行执行（`--parallel` / `-P`）

### 功能

为批量序号/范围命令新增**并行模式**，在保持默认顺序执行的同时，可通过标志并发连接多个数据库。

| 标志 | 说明 |
|------|------|
| （默认） | 顺序执行，一次一个连接 |
| `--parallel` / `-P` | 并行，默认并发数 **15** |
| `-P N` / `--parallel N` | 并行，最多 **N** 个并发（不超过批量大小） |

### 适用命令

`stats`、`report`、`query`、`schema list`、`schema describe`、`context`、`open`

### 行为

| 输出 | 行为 |
|------|------|
| stdout | 各连接结果以 `---` 分隔，**按原序号顺序**排列（非完成顺序） |
| JSON | `{ connections: [...] }` 数组按 index 排序 |
| stderr（并行） | 每行加 `[#N]` 前缀，避免交错混淆 |
| stderr（顺序） | 无额外前缀，与原先一致 |

### 示例

```bash
dbx stats 23-35              # 顺序
dbx stats 23-35 --parallel   # 并行，默认 15 并发
dbx stats 23-35 -P 3         # 并行，最多 3 并发
```

### 变更文件

| 文件 | 变更 |
|------|------|
| `packages/cli/src/cli.ts` | `runConnectionBatch`、`--parallel`/`-P` 解析、7 个批量命令重构 |
| `packages/cli/README.md` | 并行用法文档（中英文） |
| `packages/cli/dist/cli.js` | 构建产物 |

已同步至 `G:\usr\local\node_modules\@dbx-app\cli\dist\`

### 范围校验

`parseListIndexRange` **不再限制范围跨度**。`15` 为 `--parallel` 默认并发数。超过 100 个连接时 CLI 输出警告（不阻断）。

---

## 2026-07-17 — 修复范围校验：按跨度而非结束序号限制

### 问题

`dbx stats 23-35` 报错 `Range end must be <= 15. Got 35.`，但 23–35 仅 13 个连接，应在批量上限内。

### 根因

`parseListIndexRange` 错误地将 `MAX_LIST_INDEX_RANGE_END = 15` 当作结束序号上限；正确语义是**单次批量最多 15 个连接**（`end - start + 1 <= 15`），结束序号可指向列表中任意有效位置。

### 修复

| 文件 | 变更 |
|------|------|
| `packages/node-core/src/list-index.ts` | 删除 `MAX_LIST_INDEX_RANGE_END` 及 `end > 15` 检查；保留 `MAX_LIST_INDEX_RANGE_SIZE = 15` 跨度校验 |
| `packages/node-core/dist/list-index.js` / `.d.ts` | 同步构建产物 |
| `packages/cli/README.md` | 文档改为「范围跨度 ≤ 15」，移除「结束序号 ≤ 15」 |

已同步至：

- `G:\usr\local\node_modules\@dbx-app\cli\node_modules\@dbx-app\node-core\dist\`
- `C:\usr\local\node_modules\@dbx-app\mcp-server\node_modules\@dbx-app\node-core\dist\`

### 校验规则（修复后）

| 规则 | 值 |
|------|-----|
| 起始序号 | ≥ 1 |
| 结束序号 | ≥ 起始序号（无 15 上限） |
| 范围跨度 `end - start + 1` | ≤ 15 |
| 单个序号 | 任意有效索引（如 `50`） |

### 示例

| 命令 | 结果 |
|------|------|
| `dbx stats 1-15` | ✅ 15 个连接 |
| `dbx stats 23-35` | ✅ 13 个连接 |
| `dbx stats 23-37` | ✅ 15 个连接 |
| `dbx stats 1-16` | ❌ `Range size must be <= 15. Got 16 (1-16).` |
| `dbx stats 50` | ✅ 单连接 #50（若存在） |

### 验证

```powershell
dbx stats 23-35
# 应通过范围校验并尝试连接（可能因 DB 连接失败，但不再报 end <= 15）
```

---

## 2026-07-17 — 修复 CLI `resolveConnectionsByIndexRef` 重复导出崩溃

### 问题

运行任意依赖 `@dbx-app/node-core` 的 CLI 命令（如 `dbx connections list`）报错：

```
SyntaxError: Identifier 'resolveConnectionsByIndexRef' has already been declared
file:///G:/usr/local/node_modules/@dbx-app/cli/node_modules/@dbx-app/node-core/dist/connections.js:338
```

### 根因

手动 dist 同步时，`resolveConnectionsByIndexRef` 函数被**整段追加两次**（`connections.js` 第 311 行与第 338 行内容完全相同；`connections.d.ts` 声明亦重复）。源文件 `packages/node-core/src/connections.ts` 仅有一处定义，无重复。

`list-index.js` 经检查无类似重复导出。

### 修复

| 文件 | 变更 |
|------|------|
| `packages/node-core/dist/connections.js` | 删除第 337–363 行重复函数块，保留第 311–336 行 |
| `packages/node-core/dist/connections.d.ts` | 删除重复 `export declare function resolveConnectionsByIndexRef` |

已同步至：

- `G:\usr\local\node_modules\@dbx-app\cli\node_modules\@dbx-app\node-core\dist\`
- `C:\usr\local\node_modules\@dbx-app\mcp-server\node_modules\@dbx-app\node-core\dist\`
- `dbx-mcp-patches/packages/node-core/dist/`

### 验证

```powershell
dbx connections list
# exit 0 — 成功列出 52 条连接
```

---

## 2026-07-17 — CLI 连接序号范围批量（1–15）

### 变更摘要

CLI 非交互模式下，连接参数支持**序号范围**语法，一次命令按顺序对多个连接执行相同操作。MCP 仍仅支持单个序号（不变）。

### 范围语法

| 语法 | 示例 | 展开结果 |
|------|------|----------|
| 连字符 | `1-15` | 1, 2, …, 15 |
| 双点 | `1..15` | 同上 |
| 冒号 | `1:15` | 同上 |
| 带 `#` | `#1-#15`、`#3-5` | 同上 |

单个序号 `1`、`#2` 行为不变。

### 范围限制

| 规则 | 值 |
|------|-----|
| 起始序号 | ≥ 1 |
| 结束序号 | ≥ 起始序号 |
| 单次批量连接数（范围跨度） | ≤ 15 |
| 逆序范围 | 拒绝（如 `5-3`） |

超出限制返回 `INVALID_LIST_INDEX_RANGE`。

### 支持批量范围的 CLI 命令

`stats`、`report`、`query`、`schema list`、`schema describe`、`context`、`open`

- 按连接顺序依次执行
- 文本输出：多连接时用 `## #N name` 标题 + `---` 分隔
- `--json`：单连接保持原结构；多连接返回 `{ "connections": [ … ] }`
- 进度日志仍即时写 stderr（每个连接解析/连接阶段独立输出）
- **不**应用于交互式选择器；MCP 未扩展范围语法

### node-core 新增/变更

| 模块 | 导出 |
|------|------|
| `list-index.ts` | `parseListIndexRange`、`MAX_LIST_INDEX_RANGE_SIZE`、`ListIndexRangeError` |
| `connections.ts` | `resolveConnectionsByIndexRef` |

### 修改文件

- `packages/node-core/src/list-index.ts`
- `packages/node-core/src/connections.ts`
- `packages/node-core/dist/list-index.js` / `.d.ts`
- `packages/node-core/dist/connections.js` / `.d.ts`
- `packages/cli/src/cli.ts`
- `packages/cli/dist/cli.js`
- `packages/cli/README.md`
- `update_log.md`

### 示例

```bash
dbx stats 1-15          # 连接 #1 到 #15 依次 stats
dbx report 3-5          # 连接 #3、#4、#5
dbx query 1 "select 1"  # 单连接不变
dbx schema list 1..3
```

---


### 根因

`packages/cli/src/cli.ts` 的 `runCli()` 用自定义 `sink` 把 `[dbx]` 行推入 `progressLogs[]`，仅在 `succeed()` / `fail()` 返回时通过 `CliRunResult.stderr` 一次性写出；`main()` 又在命令结束后才 `process.stderr.write(result.stderr)`，导致所有进度行在命令结束时才出现。

`connection-log.ts` 的 `defaultSink` 本身已支持即时 `stderr.write`；MCP 的 `startConnectionLogCollector` 缓冲行为正确，无需改动。

### 修复

| 文件 | 变更 |
|------|------|
| `packages/node-core/src/connection-log.ts` | 新增 `stderrStreamSink()`、`cliConnectionLogOptions()`；`writeStderrLine()` 在非 TTY 下调用 `setBlocking(true)` 减少 Windows 管道缓冲 |
| `packages/cli/src/cli.ts` | 改用 `cliConnectionLogOptions()` 即时写 stderr；移除 `progressLogs` 收集与 `result.stderr` 中的进度前缀（错误信息仍经 `fail()` 输出） |

- **CLI**：每条 `connectionLog()` 立即写 stderr（真实时流逝输出）
- **MCP**：仍用 collector 缓冲，响应正文前 prepend

### 验证

`dbx report 11`（代理连接，约 3.7s）：

- 首条 stderr：**80ms**（命令总时长 3666ms）→ `streamedBeforeEnd: true`
- 进度行时间戳分散：80 → 113 → 124 → 924 → 2101 → 2879 → 3653 ms

`report` / `stats` / `query` 共用 `runCli()` + `connectionLog`，均已实时流式。

### 已安装 dist 同步

- `packages/node-core/dist/connection-log.js`、`.d.ts`
- `packages/cli/dist/cli.js`
- `C:\usr\local\node_modules\@dbx-app\mcp-server\node_modules\@dbx-app\node-core\dist\`
- `G:\usr\local\node_modules\@dbx-app\cli\dist\` 及 node-core dist

---

## 2026-07-16 — 术语：「流逝日志」→「流逝输出」

用户可见文档统一将连接反馈功能称为 **流逝输出**（streaming output），不再使用「连接进度日志」「进度日志」「日志反馈」等表述。技术模块名 `connection-log.ts` 保持不变。

---

## 2026-07-16 — 流逝输出（CLI / MCP / Web API）

### 变更摘要

为 CLI、MCP Server 和 Web 后端 API 调用统一接入 `connection-log` 模块，在连接数据库时输出代理步骤、隧道阶段和清晰错误信息，便于调试代理/SSH 隧道连接问题。

### node-core 新增 / 扩展

| 模块 | 导出 |
|------|------|
| `connection-log.ts` | `connectionLog`、`withConnectionStage`、`startConnectionLogCollector`、`mcpConnectionLogOptions`、`prependConnectionProgress` |

`database.ts`、`connections.ts`、`database-stats.ts`、`database-report.ts` 在连接各阶段调用 `connectionLog` / `withConnectionStage`。

### MCP 行为

会连接数据库的工具在响应正文前附带 `[dbx]` 进度段，与结果之间用 `---` 分隔。出错时进度段保留，便于定位失败阶段。

| 环境变量 | 默认 | 作用 |
|----------|------|------|
| `DBX_MCP_QUIET=1` | 关 | 抑制工具响应中的流逝输出 |
| `DBX_MCP_VERBOSE=1` | 关 | 显示 verbose-only 步骤（复用隧道、bridge 端点等） |

### CLI 行为

- `--quiet` / `-q` 或 `DBX_QUIET=1`：抑制进度
- `--verbose` / `-v` 或 `DBX_VERBOSE=1`：显示 verbose 步骤
- 进度输出到 **stderr**，查询结果仍在 stdout

### Web / External API

`web-backend.ts` 的 `ensureConnected()` 在调用 `/api/connection/connect` 前后记录进度。Web 模式下的实际隧道建立发生在 DBX Web 服务端；node-core 层记录客户端可见的 API 连接阶段，供 MCP Web 模式消费。

### 修改文件

- `packages/node-core/src/connection-log.ts`（新建/扩展）
- `packages/node-core/src/database.ts`、`connections.ts`、`database-stats.ts`、`database-report.ts`
- `packages/node-core/src/web-backend.ts`
- `packages/node-core/src/index.ts`
- `packages/mcp-server/src/index.ts`
- `packages/cli/src/cli.ts`
- `packages/mcp-server/README.md`

**已安装 dist 同步：**

- `C:\usr\local\node_modules\@dbx-app\mcp-server\dist\index.js`
- `C:\usr\local\node_modules\@dbx-app\mcp-server\node_modules\@dbx-app\node-core\dist\`
- `G:\usr\local\node_modules\@dbx-app\cli\dist\` 及 node-core dist

---

## 2026-07-16 — 数据库完整报告 `dbx report` / `dbx_get_database_report`

### 变更摘要

在 `dbx stats` / `dbx_get_database_stats` 基础上新增 **完整数据库报告**，所有数据来自系统目录视图，不执行 `COUNT(*)` 或慢速全表扫描。复用 stats 的排序逻辑、系统库排除与多库/Schema 作用域。

### 新增工具 / 命令

| CLI | MCP 工具 |
|-----|----------|
| `dbx report <connection\|#> [--schema] [--database] [--json]` | `dbx_get_database_report` |

### 报告章节

1. **Database Summary** — 库名/字符集/排序规则/表数量（来自 `buildCatalogSummarySql`）
2. **Tables** — 表元数据 + 注释；按行数降序（复用 `formatStatsOverviewTable` / `sortStatsRows`）
3. **Column Comments** — 非空列注释
4. **Indexes** — MySQL `STATISTICS`、PostgreSQL `pg_indexes`、SQLite `sqlite_master`

### node-core 新增

| 模块 | 导出 |
|------|------|
| `packages/node-core/src/database-report.ts` | `fetchDatabaseReport`、`buildCatalogIndexSql`、`buildCatalogColumnCommentsSql` |

### 修改文件

**上游：** `database-report.ts`（新建）、`node-core/index.ts`、`mcp-server/index.ts`、`cli/cli.ts`、README

**已安装 dist：** mcp-server、node-core、cli dist 手动同步

### 验证

```bash
dbx report 1
dbx report my-postgres --schema public --json
```

---

## 2026-07-16 — `dbx stats` 按行数降序排序 & 排除系统库

### 变更摘要

`dbx stats <connection>` 与 MCP `dbx_get_database_stats` 的输出现在**按行数（Rows est.）降序排列**（行数最多的表在前）。未指定 `--database` / `--schema` 时，自动汇总**所有用户库/Schema**，并排除系统目录。

### 排序逻辑

- 在 `formatStatsOverviewTable()` 输出前调用 `sortStatsRows()`，全局按行数降序排序
- 兼容多种行数字段：`rows_estimate`、`TABLE_ROWS`、`n_live_tup`、`reltuples`、`count`、`nrecords` 等
- `null` / 未知行数视为最低优先级（排在最后）

### 系统库排除规则

| 数据库类型 | 排除范围 |
|-----------|---------|
| MySQL 系 | `information_schema`、`mysql`、`performance_schema`、`sys` |
| PostgreSQL 系 | `information_schema`、`pg_catalog`、`pg_toast` |
| SQLite | 仅排除 `sqlite_%` 内部表（原有逻辑） |
| 通用 information_schema | 同上 MySQL + PostgreSQL 系统 schema |

### 作用域行为

- **MySQL**：未传 `--database` → 查询所有用户库，输出增加 `Database` 列；传 `--database` → 仅该库
- **PostgreSQL**：未传 `--schema` → 查询所有用户 schema，输出增加 `Schema` 列；传 `--schema` → 仅该 schema
- **MongoDB**：集合按 `count` / `nrecords` 降序排列

### 修改文件

**上游（`C:\usr\local\dbx`）：**

- `packages/node-core/src/database-stats.ts` — `sortStatsRows()`、系统库过滤、全用户库 SQL、`CatalogStatsScope`
- `packages/node-core/dist/database-stats.js`
- `packages/mcp-server/src/index.ts` — 改用 `fetchDatabaseStats()`，移除重复 stats 代码

**已安装包：**

- `C:\usr\local\node_modules\@dbx-app\mcp-server\dist\index.js`
- `C:\usr\local\node_modules\@dbx-app\mcp-server\node_modules\@dbx-app\node-core\dist\database-stats.js`
- `G:\usr\local\node_modules\@dbx-app\cli\node_modules\@dbx-app\node-core\dist\database-stats.js`

### 验证

```bash
dbx stats pc-bp127s4ad63890doi_mysql_polardb_resource
# 首行 device_heart_status_history Rows (est.) = 5208176，依次递减
```

---

## 2026-07-16 — 列表序号 `#` 与数字引用

### 变更摘要

为 MCP Server 与 CLI 的列表输出增加 **`#` 列**（1-based 行序号），并允许在连接/代理参数中使用 `1`、`#2` 等数字代替 UUID 或名称。

### 解析规则

1. **列表顺序**：与 `loadConnections()` / `loadTunnelProfiles()`（`ORDER BY rowid`）返回顺序一致；MCP scoped 模式使用 scoped 连接列表顺序。
2. **解析优先级**：精确 **UUID/ID** → **名称**（唯一时）→ **列表序号**（`1` 或 `#1` 格式）。
3. **序号含义**：每次 list 重新加载；`N` 表示当前列表第 N 行，非持久化 ID。
4. **代理序号**：仅在 `type === "proxy"` 的配置中计数（与 `dbx_list_proxies` / `dbx proxies list` 一致）。

### 带 `#` 列的列表

| 命令 | MCP 工具 |
|------|----------|
| `dbx connections list` | `dbx_list_connections` |
| `dbx proxies list` | `dbx_list_proxies` |
| `dbx schema list` / `dbx_list_tables` | 表列表增加 `#` 列 |

### 支持数字引用的命令

**MCP：** 所有带 `connection_id` / `connection_name` 的工具；`dbx_add_connection` 的 `proxy_profile_id` / `proxy_profile_name`；`dbx_remove_connection`。

**CLI：** `stats`、`query`、`schema list/describe`、`context`、`open`；`connections add --proxy-profile-id 1`。

### node-core 新增

| 模块 | 导出 |
|------|------|
| `packages/node-core/src/list-index.ts` | `parseListIndex`、`resolveConnectionByIndex`、`resolveProxyProfileByIndex`、`listProxyProfiles` |

`findProxyProfile()` 已扩展：ID/名称未命中时尝试序号解析。

### 修改文件

**上游（`C:\usr\local\dbx`）：**

- `packages/node-core/src/list-index.ts`（新建）
- `packages/node-core/src/index.ts`
- `packages/node-core/src/tunnel-profiles.ts`
- `packages/mcp-server/src/index.ts`
- `packages/mcp-server/src/tunnel-profiles.ts`
- `packages/mcp-server/README.md`
- `packages/cli/src/cli.ts`
- `packages/cli/src/cli-format.ts`

**已安装 dist（手动同步）：**

- `C:\usr\local\node_modules\@dbx-app\mcp-server\dist\index.js`
- `C:\usr\local\node_modules\@dbx-app\mcp-server\dist\tunnel-profiles.js`
- `C:\usr\local\node_modules\@dbx-app\mcp-server\node_modules\@dbx-app\node-core\dist/list-index.js` 等
- `G:\usr\local\node_modules\@dbx-app\cli\dist/cli.js`、`cli-format.js` 及 node-core dist

### 示例

```bash
dbx connections list          # 输出含 # | ID | Name | ...
dbx stats 1                   # 使用第 1 个连接
dbx query 2 "select 1"        # 使用第 2 个连接
dbx connections add ... --proxy-profile-id 1
```

MCP：`connection_name: "1"` 或 `connection_id: "#2"`；`proxy_profile_id: "1"`。

---

## 2026-07-16 — CLI 同步 MCP 新功能（代理配置 / 数据库状态概览）

### 变更摘要

将 MCP Server 近期新增能力同步到 **`@dbx-app/cli`**，便于在终端测试，无需 MCP 客户端。

### 新增 CLI 命令 / 参数

#### `dbx connections add`

| 参数 | 说明 |
|------|------|
| `--name` / `--type` / `--host` | 必填 |
| `--port` / `--username` / `--password` / `--database` / `--ssl` / `--driver-profile` | 可选连接字段 |
| `--proxy` | 启用内联 SOCKS5/HTTP 代理 |
| `--proxy-type` | `socks5`（默认）或 `http` |
| `--proxy-host` / `--proxy-port` / `--proxy-username` / `--proxy-password` | 内联代理参数 |
| `--proxy-profile-id` | 引用已保存代理配置 ID（与内联代理互斥） |
| `--proxy-profile-name` | 引用已保存代理配置名称 |

校验规则与 MCP `dbx_add_connection` 一致：`PROXY_CONFLICT`、`AMBIGUOUS_PROXY_PROFILE`、`PROXY_PROFILE_NOT_FOUND`。

#### `dbx proxies list`

对应 MCP `dbx_list_proxies`，列出 DBX **设置 > 隧道** 中 `type === "proxy"` 的配置。

#### `dbx stats <connection>`

对应 MCP `dbx_get_database_stats`，从 `information_schema` / `pg_catalog` / `sqlite_master` 等系统目录读取表级元数据与统计估计值。支持 `--schema`、`--database`、`--json`。

### node-core 共享逻辑

| 模块 | 内容 |
|------|------|
| `packages/node-core/src/database-stats.ts` | **新增** — `fetchDatabaseStats()`、目录 SQL 构建、Redis/Mongo 分支 |
| `packages/node-core/src/tunnel-profiles.ts` | **扩展** — `hasProxyProfileRef`、`hasInlineProxyParams`、`findProxyProfile`、`findProxyProfilesByName` |

CLI 与 MCP 均可复用上述模块；MCP 仍保留 Web 模式专用的 `tunnel-profiles.ts` 包装。

### 修改文件

**上游（`C:\usr\local\dbx`）：**

- `packages/cli/src/cli.ts` — 新命令与 flag 解析（新建 CLI 源码包）
- `packages/cli/src/cli-format.ts`
- `packages/cli/package.json` / `tsconfig.json` / `README.md`
- `packages/cli/dist/cli.js` / `cli-format.js`
- `packages/node-core/src/database-stats.ts`
- `packages/node-core/src/tunnel-profiles.ts`
- `packages/node-core/src/index.ts`
- `packages/node-core/package.json`
- `packages/node-core/dist/` — `database-stats.js`、`tunnel-profiles.js`、`backend.js`、`index.js`

**已安装 CLI（`G:\usr\local\node_modules\@dbx-app\cli`）：**

- `dist/cli.js` / `dist/cli-format.js`
- `package.json` — 版本 `0.4.29`，依赖 `@dbx-app/node-core@^0.4.29`
- `node_modules/@dbx-app/node-core/dist/` — 同步 node-core dist

### 测试示例

```bash
# 列出已保存代理配置
dbx proxies list

# 内联代理添加连接
dbx connections add --name pg-via-proxy --type postgres --host 10.0.0.5 --port 5432 \
  --username app --password secret --proxy --proxy-host 127.0.0.1 --proxy-port 1080

# 引用已保存代理
dbx connections add --name pg-via-profile --type postgres --host 10.0.0.5 --port 5432 \
  --proxy-profile-name "My SOCKS5"

# 数据库状态概览
dbx stats my-postgres --schema public
dbx stats my-mysql --json

# 验证连接已写入
dbx connections list
```

CLI 入口：`G:\usr\local\dbx.cmd` → `G:\usr\local\node_modules\@dbx-app\cli\dist\cli.js`

### 已知限制

1. 未执行编译/测试（按用户要求）；dist 为手工同步，发布前请在 monorepo 运行 `pnpm build:packages`
2. 上游 checkout 原先缺少 `packages/cli/`，本次已重建源码目录
3. `dbx connections add` 成功后调用 `notifyReload()` 通知 DBX 桌面刷新（需 DBX 运行）

---


### 变更摘要

新增 MCP 工具 **`dbx_get_database_stats`**（STAT），从系统目录视图读取数据库/表级元数据与统计估计值，供 AI 快速了解库内对象规模，**不执行手动 `COUNT(*)`**，也不维护缓存计数。

### 工具名称

`dbx_get_database_stats`（与现有 `dbx_get_schema_context` 命名风格一致）

### 参数

| 参数 | 说明 |
|------|------|
| `connection_id` | 连接 ID（可选，与 `connection_name` 二选一或配合 scope） |
| `connection_name` | 连接名称 |
| `database` | 数据库名（Dameng 亦作 schema 别名） |
| `schema` | Schema 名（PostgreSQL 默认 `public`，SQL Server 默认 `dbo`，Dameng 默认登录用户） |

### 各数据库类型查询策略

| 类型 | 目录来源 |
|------|----------|
| **MySQL / MariaDB / Doris / StarRocks / Manticore** | `information_schema.TABLES`（`TABLE_ROWS`、`DATA_LENGTH`、`INDEX_LENGTH`、`ENGINE`、`TABLE_COMMENT`）+ `information_schema.SCHEMATA` 摘要 |
| **PostgreSQL 系**（postgres、redshift、gaussdb、kwdb、opengauss、kingbase、highgo、vastbase、dameng） | `information_schema.tables` + `pg_catalog.pg_class` + `pg_stat_user_tables.n_live_tup` + `pg_relation_size` / `pg_total_relation_size` |
| **SQLite / rqlite** | `sqlite_master`（表/视图列表；无行数/大小目录字段） |
| **MongoDB** | `listTables` + 各集合 `db.<collection>.stats()`（collStats，最多 50 个集合） |
| **Redis** | `INFO` + `DBSIZE`（内存、keyspace 摘要） |
| **其他 SQL（经 bridge）** | `information_schema.tables` 尽力回退（仅 name/type） |
| **不支持** | elasticsearch、neo4j、cassandra、向量库等 → `UNSUPPORTED_DB_TYPE` |

### 输出格式

Markdown 表格，列：**Name、Type、Engine、Rows/Docs (est.)、Data、Index、Total、Comment**。MySQL/PostgreSQL 含可选 Summary 段（库名、字符集、库大小等）。

### 修改文件

**上游（`C:\usr\local\dbx`）：**

- `packages/mcp-server/src/index.ts` — 新增工具与目录查询辅助函数
- `packages/mcp-server/README.md` — 英文/中文文档

**本地安装包（`C:\usr\local\node_modules\@dbx-app\mcp-server`）：**

- `dist/index.js` — 与上游逻辑同步
- `README.md` — 与上游同步
- `update_log.md` — 本条目

### 已知限制

1. MongoDB 集合 stats 单次最多 50 个（避免 MCP 超时）
2. SQLite 系统目录不提供行数/大小估计
3. 通用 `information_schema` 回退不含 size/rows 列
4. 未修改 node-core；复用现有 `executeQuery` / `listTables` / `executeRedisCommand`
5. 未执行编译/测试（按用户要求）

---

## 2026-07-16 — 已保存代理（Tunnel Profile）支持 + 内联代理参数

### 变更摘要

为 DBX MCP Server 新增**已保存代理配置**（Settings > Tunnels）的引用能力，同时保留内联 `proxy_*` 参数。两种模式互斥，并提供 `dbx_list_proxies` 工具列出可用配置。

### 存储模型（调研结论）

DBX 将共享隧道/代理配置保存在 SQLite `dbx.db` 的 `tunnel_profiles` 表：

| 字段 | 说明 |
|------|------|
| `id` | 配置 UUID（即 `proxy_profile_id`） |
| `config_json` | JSON 序列化的 `TransportLayerConfig`（`type: "proxy"` / `"ssh"` / `"http_tunnel"`） |

连接侧通过 `transport_layers[].profile_id` 引用已保存配置；DBX 在**连接时**解析完整代理参数，MCP 只写入引用桩（不含密码副本）。

### 新增 MCP 工具

#### `dbx_list_proxies`

列出 DBX **设置 > 隧道** 中 `type === "proxy"` 的已保存配置。

| 列 | 说明 |
|----|------|
| ID | `tunnel_profiles.id` |
| Name | 配置名称 |
| Type | `socks5` / `http` |
| Host / Port / Username | 代理端点信息 |
| Enabled | 是否启用 |
| Summary | 如 `socks5://127.0.0.1:1080` |

数据来源：
- **桌面模式**：直接读取 `dbx.db` 的 `tunnel_profiles` 表
- **Web/Docker 模式**：`GET /api/tunnel-profiles/list`

### `dbx_add_connection` 新增参数

| 参数 | 类型 | 说明 |
|------|------|------|
| `proxy_profile_id` | string | 已保存代理配置 ID（与内联 `proxy_*` 互斥） |
| `proxy_profile_name` | string | 已保存代理配置名称（`proxy_profile_id` 的替代） |

### 代理配置模式（互斥）

1. **内联模式**（已有）：`proxy_enabled` + `proxy_host` + 可选 `proxy_type` / `proxy_port` / 认证字段
2. **引用已保存配置**（新增）：`proxy_profile_id` 或 `proxy_profile_name` → 写入 `transport_layers` 引用桩（含 `profile_id`）

校验规则：
- 不可同时提供内联 `proxy_*` 与 `proxy_profile_id` / `proxy_profile_name` → `PROXY_CONFLICT`
- 不可同时指定 `proxy_profile_id` 与 `proxy_profile_name` → `PROXY_CONFLICT`
- 名称匹配到多个配置 → `AMBIGUOUS_PROXY_PROFILE`（需改用 ID）
- 配置不存在 → `PROXY_PROFILE_NOT_FOUND`

### 修改文件

**上游（`C:\usr\local\dbx`）：**

- `packages/mcp-server/src/index.ts` — `dbx_list_proxies`、`proxy_profile_id/name`、互斥校验
- `packages/mcp-server/src/tunnel-profiles.ts` — 读取 `tunnel_profiles`（桌面 + Web API）
- `packages/mcp-server/README.md` — 英文/中文文档
- `packages/node-core/src/tunnel-profiles.ts` — 可选 Backend 层读取（上游 PR 用）
- `packages/node-core/src/connections.ts` — `profile_id` 字段
- `packages/node-core/src/backend.ts` / `web-backend.ts` — `loadTunnelProfiles` Backend 方法

**本地安装包（`C:\usr\local\node_modules\@dbx-app\mcp-server`）：**

- `dist/index.js` — 与上游逻辑同步
- `dist/tunnel-profiles.js` — 隧道配置读取模块
- `README.md` — 与上游同步

### 已知限制

1. 仅支持引用 **proxy** 类型的 tunnel profile（不含 SSH / HTTP script tunnel）
2. SSH 隧道参数仍未暴露给 MCP
3. 未执行编译/测试（按用户要求）

---

## 2026-07-16 — `dbx_add_connection` 内联代理参数（初版）

为 `dbx_add_connection` 新增 SOCKS5/HTTP 内联代理参数，透传至 `addConnection()` 遗留 `proxy_*` 字段。

---

## 2026-07-16 — 修复 CLI `dbx proxies list` 模块缺失错误

### 问题

运行 `dbx proxies list` 报错：

```
Error [ERR_MODULE_NOT_FOUND]: Cannot find module '...\node-core\dist\production-safety.js'
```

根因：CLI 安装的 `@dbx-app/node-core` dist 不完整（28 个文件），`index.js` 导出了 15 个模块，但缺少 2 个 `.js` 及对应 `.d.ts`；另有 11 个文件与 MCP 版本不一致（如 `connections.js` 缺少 `removeConnectionById` 导出）。

### 缺失 / 不一致文件

**完全缺失（4 个）：**

| 文件 | 说明 |
|------|------|
| `production-safety.js` / `.d.ts` | 生产环境 SQL 安全评估 |
| `sql-risk.js` / `.d.ts` | SQL 风险分类（production-safety 依赖） |

**版本不一致（11 个，已从 MCP 同步）：**

`backend.d.ts`、`connections.js` / `.d.ts`、`database.js` / `.d.ts`、`diagnostics.js` / `.d.ts`、`index.d.ts`、`sql-safety.js`、`web-backend.js` / `.d.ts`

### 修复操作

从 `C:\usr\local\node_modules\@dbx-app\mcp-server\node_modules\@dbx-app\node-core\dist` 复制缺失文件并同步差异文件至：

`G:\usr\local\node_modules\@dbx-app\cli\node_modules\@dbx-app\node-core\dist`

上游 `C:\usr\local\dbx\packages\node-core\dist` 仅含 4 个文件，不作为同步源。MCP node-core 已完整（32 个文件），无需额外同步。

CLI 自身 `dist/cli.js` 仅依赖 `@dbx-app/node-core` 与 `./cli-format.js`，后者存在，无需修改。

### 验证

```powershell
dbx proxies list
# exit 0 — 成功列出 2 条 proxy 配置
```

修复后 CLI 与 MCP 的 node-core dist 哈希完全一致（各 32 个文件）。
