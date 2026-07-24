---
name: connection-url
description: >-
  Extract host, port, username, password, database, and DB type from messy pasted
  text, then emit one connection URL per line (plain links only, no tables). Use
  when the user pastes credentials or asks to 生成链接 / make connection strings.
---

# 粘贴文本 → 提取凭证 → 生成连接 URL

用户粘贴一段杂乱文本（聊天记录、配置、备注、OCR 等）。你的任务：

1. **分析**出每条连接的：类型、主机、端口、用户名、密码、数据库、其它参数  
2. **按格式生成**标准连接 URL（明文，不脱敏）  
3. **只输出纯链接**：一行一个 URL，不要表格、不要字段说明、不要代码围栏（除非用户另要求）

不要去读仓库源码；本文件已含全部生成格式。

## 输出格式

```
mysql://root:p%40ss@10.0.0.8:3306/shop
postgresql://alice:secret@db.example.com:5432/app?sslmode=require
```

- 一行一个；有几条连几行。
- 密码做百分号编码（`@`→`%40`，`:`→`%3A`，`/`→`%2F`，`#`→`%23`，空格→`%20` 等）。
- 缺端口用下表默认端口；缺字段则省略对应段，仍尽量生成可用 URL。

## 从文本怎么抽

在杂文里找这些线索（中英都认）：

| 字段 | 常见写法 |
|------|----------|
| 类型 | MySQL、Postgres、PG、Redis、Mongo、Oracle、SQL Server、达梦、人大金仓、ClickHouse、TiDB… |
| 主机 | host、主机、地址、ip、server、`xxx.xxx.xxx.xxx`、域名 |
| 端口 | port、端口、`:3306` |
| 用户 | user、username、用户、账号、帐号、uid |
| 密码 | password、passwd、pwd、密码、口令 |
| 数据库 | database、db、库名、schema（按语境） |
| 名称 | 连接名、备注名、环境名（prod/测试） |

也可识别已是半成品的 `user:pass@host:port/db`、`jdbc:…`、`mysql://…` —— 拆开字段后**重新**按标准格式生成一条干净 URL。

## 生成格式（按类型）

密码段用编码后的值，下表里写作 `{pass}`。

| 类型 | 默认端口 | 生成 URL |
|------|----------|----------|
| MySQL / MariaDB / TiDB | 3306 | `mysql://{user}:{pass}@{host}:{port}/{db}` |
| PostgreSQL | 5432 | `postgresql://{user}:{pass}@{host}:{port}/{db}` |
| Redshift | 5439 | `redshift://{user}:{pass}@{host}:{port}/{db}` |
| Redis | 6379 | `redis://{user}:{pass}@{host}:{port}/{db}`；TLS 用 `rediss://` |
| MongoDB | 27017 | `mongodb://{user}:{pass}@{host}:{port}/{db}`；SRV 用 `mongodb+srv://{user}:{pass}@{host}/{db}` |
| SQL Server | 1433 | `jdbc:sqlserver://{host}:{port};databaseName={db};user={user};password={pass}` |
| Oracle（Service） | 1521 | `jdbc:oracle:thin:@//{host}:{port}/{service}` |
| Oracle（SID） | 1521 | `jdbc:oracle:thin:@{host}:{port}:{sid}` |
| 达梦 Dameng | 5236 | `dm://{user}:{pass}@{host}:{port}/{db}` |
| KingBase | 54321 | `kingbase8://{user}:{pass}@{host}:{port}/{db}` |
| KWDB | 26257 | `kwdb://{user}:{pass}@{host}:{port}/{db}` |
| GaussDB / openGauss | 5432 | `gaussdb://` 或 `opengauss://{user}:{pass}@{host}:{port}/{db}` |
| ClickHouse | 8123 | `clickhouse://{user}:{pass}@{host}:{port}/{db}`；HTTPS 可 `https://{user}:{pass}@{host}:{port}/{db}` |
| TDengine | 6041 | `jdbc:TAOS-WS://{user}:{pass}@{host}:{port}/{db}` |
| XuguDB | 5138 | `jdbc:xugu://{user}:{pass}@{host}:{port}/{db}` |
| IoTDB | 6667 | `jdbc:iotdb://{user}:{pass}@{host}:{port}` |
| GBase 8s | 9088 | `jdbc:gbasedbt-sqli://{user}:{pass}@{host}:{port}/{db}` |
| Informix | 9088 | `jdbc:informix-sqli://{user}:{pass}@{host}:{port}/{db}:INFORMIXSERVER={server}` |
| Elasticsearch | 9200 | `https://{host}:{port}`（需标明 elasticsearch） |
| etcd | 2379 | `etcd://{host}:{port}` |
| ZooKeeper | 2181 | `zookeeper://{host}:{port}` |

可选 query：

- MySQL SSL：`?ssl-mode=required` 或 `?charset=utf8mb4`
- Postgres SSL：`?sslmode=require`
- 连接显示名：`?name=公司-本地`（值做 URL 编码）
- Mongo：`?authSource=admin&replicaSet=rs0`

用户指定要 JDBC 时：在对应 scheme 前加 `jdbc:`（如 `jdbc:mysql://…`、`jdbc:postgresql://…`）。

## 示例

**粘贴：**
```
测试库 mysql
地址 10.0.0.8 端口 3306
账号 root 密码 p@ss
库名 shop
```

**输出：**

```
mysql://root:p%40ss@10.0.0.8:3306/shop
```

**粘贴：**
```
pg 生产
host=db.example.com
user=alice
password=secret
database=app
sslmode=require
```

**输出：**

```
postgresql://alice:secret@db.example.com:5432/app?sslmode=require
```
