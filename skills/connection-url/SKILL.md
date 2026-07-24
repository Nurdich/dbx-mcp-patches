---
name: connection-url
description: >-
  Parse database connection URLs / JDBC / DSN into a plain field list with real
  credentials (no redaction). Use when the user pastes connection strings or asks
  to extract host, port, user, password, database from URLs.
---

# Connection URL → 明文列表

从用户粘贴的连接串提取字段，**直接输出明文列表，不脱敏、不读其它文件**。下面规则与样例已自包含。

## 输出格式

```markdown
| # | 名称 | 类型 | 主机 | 端口 | 用户 | 密码 | 数据库 | SSL | 参数 | URL |
|---|------|------|------|------|------|------|--------|-----|------|-----|
| 1 | — | PostgreSQL (`postgres`) | db.example.com | 5433 | alice | secret | app | 是 | sslmode=require | postgresql://alice:secret@… |
```

缺项填 `—`。多条 URL 多行。密码原样写出。

## Scheme → 类型 / 默认端口

| Scheme | dbType | driverProfile | 显示名 | 默认端口 |
|--------|--------|---------------|--------|----------|
| mysql | mysql | mysql | MySQL | 3306 |
| mariadb | mysql | mariadb | MariaDB | 3306 |
| postgres / postgresql | postgres | postgres | PostgreSQL | 5432 |
| redshift | redshift | redshift | Redshift | 5439 |
| redis / rediss | redis | redis | Redis | 6379 |
| etcd | etcd | etcd | etcd | 2379 |
| zookeeper | zookeeper | zookeeper | Apache ZooKeeper | 2181 |
| mongodb / mongodb+srv | mongodb | mongodb | MongoDB | 27017 |
| clickhouse | clickhouse | clickhouse | ClickHouse | 8123 |
| sqlserver / mssql | sqlserver | sqlserver | SQL Server | 1433 |
| oracle | oracle | oracle | Oracle | 1521 |
| elasticsearch | elasticsearch | elasticsearch | Elasticsearch | 9200 |
| qdrant | qdrant | qdrant | Qdrant | 6333 |
| milvus | milvus | milvus | Milvus | 19530 |
| weaviate | weaviate | weaviate | Weaviate | 8080 |
| chromadb | chromadb | chromadb | ChromaDB | 8000 |
| dm / dameng | dameng | dm | 达梦 Dameng | 5236 |
| kingbase / kingbase8 | kingbase | kingbase | KingBase | 54321 |
| gaussdb | gaussdb | gaussdb | GaussDB | 5432 |
| opengauss | gaussdb | opengauss | openGauss | 5432 |
| kwdb | kwdb | kwdb | KWDB | 26257 |
| gbase | gbase | gbase | GBase | 5258 |
| gbasedbt-sqli | gbase | gbase8s | GBase 8s | 9088 |
| informix-sqli | informix | informix | Informix | 9088 |
| yashandb | yashandb | yashandb | YashanDB | 1688 |
| questdb | questdb | questdb | QuestDB | 8812 |
| tdengine / taos-ws | tdengine | tdengine | TDengine | 6041 |
| oscar | oscar | oscar | 神通 OSCAR | 2003 |
| xugu | xugu | xugu | XuguDB | 5138 |
| iotdb | iotdb | iotdb | Apache IoTDB | 6667 |
| iris | iris | iris | IRIS | 1972 |
| jdbc:h2:… | h2 | h2 | H2 | 0（文件）或 TCP 端口 |
| jdbc:ucanaccess:… | access | access | Microsoft Access | 0 |
| jdbc:dremio:… | jdbc | dremio | Dremio | 31010 / zk 2181 |
| jdbc:arrow-flight-sql:… | jdbc | dremio | Dremio | 32010 |

`http`/`https` 必须带 profile：`clickhouse` | `elasticsearch` | `qdrant` | `milvus` | `weaviate` | `chromadb`。`jdbc:` 前缀可剥掉后按内层 scheme 解析（特殊 JDBC 见样例）。

## 通用规则

- URL 解码：`%40`→`@`，中文 name 等
- `?name=` / `?Name=` → 名称列，并从参数中去掉
- 无端口 → 用上表默认端口
- SSL=是：`rediss`、`https`、`mongodb+srv`；MySQL `ssl-mode=required|require|verify_ca|verify_identity` 或 `require_ssl=true`；Postgres/KWDB `sslmode=require|verify-ca|verify-full`；主机以 `.tidbcloud.com` 结尾
- 未知 scheme（如 `ftp`）→ 报错不支持
- **不脱敏**

## 契约样例（输入 → 提取结果）

按这些样例照搬提取；同类 URL 类推。

### Postgres / KWDB / Dameng / KingBase

| 输入 | 结果要点 |
|------|----------|
| `postgresql://alice:secret@db.example.com:5433/app?sslmode=require` | postgres, host db.example.com, 5433, alice/secret, app, ssl=是, params sslmode=require |
| `kwdb://root:secret@kw.example.com/defaultdb?sslmode=require` | kwdb, 默认端口 26257, ssl=是 |
| `dm://SYSDBA:password@127.0.0.1:5236/DAMENG` | dameng / profile dm / 达梦 Dameng |
| `kingbase://…` / `kingbase8://…` / `jdbc:kingbase8://…` + `framework:secret@172.21.203.70:443/hq_official?sslmode=disable` | kingbase, 443, ssl=否 |
| `kingbase8://172.21.203.70:443/hq_official`（无账号） | 主机/库有值；若合并已有表单则保留原用户密码 |

### MySQL

| 输入 | 结果要点 |
|------|----------|
| `mysql://root:p%40ss@127.0.0.1/shop?charset=utf8mb4` | 密码 `p@ss`, 端口 3306, params charset=utf8mb4 |
| `mysql://root:123456@localhost/?name=%E5%85%AC%E5%8F%B8+-+%E6%9C%AC%E5%9C%B0Docker&charset=utf8mb4` | 名称=`公司 - 本地Docker`, params 仅 charset=utf8mb4 |
| 仅 `?name=…` | 名称有值, params 空 |
| `mysql://root@localhost/app?Name=Analytics+Local&ssl-mode=required` | 名称 Analytics Local, ssl=是, params ssl-mode=required |
| `?ssl-mode=required` 或 `?require_ssl=true` | ssl=是 |
| `mysql://…@….tidbcloud.com:4000/test` | ssl=是 |
| `jdbc:mysql://127.0.0.1:1234/example?user=admin&password=pwd&useUnicode=true&characterEncoding=UTF8&useSSL=false` | 用户 admin 密码 pwd（从 query 提起）, params 去掉 user/password |
| `jdbc:mysql://…?user=xxxxx%40db_readonly%40127.0.0.1&password=p%40wd&useSSL=false` | 用户 `xxxxx@db_readonly@127.0.0.1`, 密码 `p@wd` |
| `mysql://127.0.0.1:1234/example?user=admin&password=pwd&charset=utf8mb4`（非 JDBC） | 用户/密码为空；params 原样保留含 user/password |

### Redis / 通用 JDBC 内层

| 输入 | 结果要点 |
|------|----------|
| `rediss://default:secret@redis.example.com:6379/0#insecure` | redis, ssl=是, params `insecure=true`, database `0` |
| `jdbc:postgresql://alice:secret@db.example.com:5433/app?sslmode=require` | 同 postgres 直连 |
| `jdbc:mysql://root:p%40ss@127.0.0.1:3307/shop?charset=utf8mb4` | 密码 p@ss, 3307 |

### TDengine / Xugu / IoTDB / GBase 8s / Informix / Access

| 输入 | 结果要点 |
|------|----------|
| `jdbc:TAOS-WS://root:taosdata@td.example.com:6041/power?timezone=UTC` | tdengine, 6041 |
| `jdbc:xugu://alice:secret@xugu.example.com:5138/demo?charset=utf8` | xugu, 5138 |
| `jdbc:iotdb://root:secret@iotdb.example.com:6667?sql_dialect=table` | iotdb, 无 database, params sql_dialect=table |
| `jdbc:gbasedbt-sqli://gbasedbt:secret@gbase.example.com:20013/testdb:GBASEDBTSERVER=gbase01;CLIENT_LOCALE=zh_cn.utf8` | gbase/gbase8s, params 冒号后那段 |
| `jdbc:informix-sqli://192.168.1.1:9088/mydb:INFORMIXSERVER=ol_informix` | informix, params INFORMIXSERVER=… |
| 多参数 `;DB_LOCALE=…` | params 整段保留 |
| `jdbc:informix-sqli://user:p%40ss@db.example.com:1533/testdb:INFORMIXSERVER=myserver` | 密码 p@ss |
| 无冒号参数 | params 空 |
| `jdbc:ucanaccess:///Users/me/data/Northwind.accdb;memory=false` | access, host=文件路径, port=0, database=Northwind.accdb, URL 列保留原串 |

### SQL Server / H2 / Oracle

| 输入 | 结果要点 |
|------|----------|
| `jdbc:sqlserver://sql.example.com:1434;databaseName=erp;user=sa;password=s%40cret;encrypt=true` | sqlserver, 1434, sa/s@cret, erp, params encrypt=true, 端口显式 |
| `…\\SQLEXPRESS:1433;…` | port=1433 且显式 |
| `…\\SQLEXPRESS;databaseName=erp;…`（无端口） | port=1433 默认，非显式 |
| `jdbc:h2:split:28:C:/dbx-test/h2/sample-db;AUTO_SERVER=TRUE` | h2 文件, host=路径, port=0, user=sa, password 空, database=sample-db |
| `jdbc:h2:tcp://localhost:9123/~/sample-db;USER=sa;PASSWORD=s%40cret;MODE=MySQL` | h2 服务器, 9123, sa/s@cret, database=~/sample-db, params MODE=MySQL |
| H2 URL 无 USER/PASSWORD | 合并表单时保留已填账号 |
| H2 URL 有 USER/PASSWORD | 用 URL 里的账号 |
| `jdbc:oracle:thin:@//oracle.example.com:1522/ORCLPDB1` | service_name, ORCLPDB1 |
| `jdbc:oracle:thin:@oracle.example.com:1521:ORCL` | sid, ORCL |
| `jdbc:oracle:thin:@(DESCRIPTION=(ADDRESS=(PROTOCOL=TCP)(HOST=oracle.example.com)(PORT=1521))(CONNECT_DATA=(SERVICE_NAME=orcl)))` | host/port/service 从描述符解析, URL 列保留原串 |

### MongoDB / HTTP

| 输入 | 结果要点 |
|------|----------|
| `mongodb+srv://reader:secret@cluster.example.com/app?retryWrites=true` | mongodb, 27017, ssl=是, URL 列保留整串 |
| `mongodb://reader:pa@ss:word@mongo.example.com/admin?authSource=admin` | 密码 `pa@ss:word`；规范化串里密码为 `pa%40ss%3Aword` |
| 无效 `%` 如 `pa%ss` | 规范为 `pa%25ss` |
| `mongodb://test:test@1.1.1.1:27017,1.1.1.2:27017,1.1.1.3:27017/admin?authMechanism=SCRAM-SHA-256&authSource=admin&replicaSet=testRS0` | 主机取第一个 1.1.1.1, URL 列保留多 host 原串 |
| 无账号多 host `mongodb://host1:27017,host2:27017/?replicaSet=rs0` | 用户密码空, host=host1 |
| `https://search.example.com:9243` + profile elasticsearch | elasticsearch, 9243, ssl=是 |
| `https://default:secret@clickhouse.example.com:8443/default?secure=true` + clickhouse | clickhouse, default/secret, ssl=是 |
只返回 URL，不要输出其他内容。一行一个
### 拒绝

`ftp://example.com` → 不支持的 scheme。
