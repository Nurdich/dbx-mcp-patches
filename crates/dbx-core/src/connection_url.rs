//! Parse database connection URLs into connection fields.
//!
//! Mirrors the desktop `apps/desktop/src/lib/connection/connectionUrl.ts` scheme table
//! and common `scheme://user:pass@host:port/db` parsing. JDBC special forms are partially
//! supported (sqlserver / oracle thin / gbase8s / informix / dremio).
//!
//! A leading `jdbc:` prefix (e.g. `jdbc:mysql://`, `jdbc:postgresql://`, `jdbc:mariadb://`)
//! is stripped before scheme matching so JDBC URLs align with the native schemes.
//!
//! If the password contains `@`, encode it as `%40` in the URL. Authority parsing uses the
//! last `@` as the userinfo/host separator (RFC-style); unencoded `@` in passwords is
//! ambiguous across tools — prefer percent-encoding rather than relying on heuristics.

use crate::models::connection::DatabaseType;
use percent_encoding::percent_decode_str;

/// Result of parsing a database connection URL / DSN.
#[derive(Debug, Clone, PartialEq)]
pub struct ParsedConnectionUrl {
    pub name: Option<String>,
    pub db_type: DatabaseType,
    pub driver_profile: String,
    pub driver_label: String,
    pub host: String,
    pub port: u16,
    pub username: String,
    pub password: String,
    pub database: Option<String>,
    pub url_params: String,
    pub ssl: bool,
    pub connection_string: Option<String>,
    pub oracle_connection_type: Option<String>,
    pub use_mongo_url: bool,
    pub port_explicit: bool,
}

#[derive(Clone, Copy)]
struct SchemeProfile {
    db_type: DatabaseType,
    profile: &'static str,
    label: &'static str,
    default_port: u16,
}

fn scheme_profile(scheme: &str) -> Option<SchemeProfile> {
    Some(match scheme {
        "mysql" => SchemeProfile {
            db_type: DatabaseType::Mysql,
            profile: "mysql",
            label: "MySQL",
            default_port: 3306,
        },
        "mariadb" => SchemeProfile {
            db_type: DatabaseType::Mysql,
            profile: "mariadb",
            label: "MariaDB",
            default_port: 3306,
        },
        "postgres" | "postgresql" => SchemeProfile {
            db_type: DatabaseType::Postgres,
            profile: "postgres",
            label: "PostgreSQL",
            default_port: 5432,
        },
        "redshift" => SchemeProfile {
            db_type: DatabaseType::Redshift,
            profile: "redshift",
            label: "Redshift",
            default_port: 5439,
        },
        "redis" | "rediss" => SchemeProfile {
            db_type: DatabaseType::Redis,
            profile: "redis",
            label: "Redis",
            default_port: 6379,
        },
        "etcd" => SchemeProfile {
            db_type: DatabaseType::Etcd,
            profile: "etcd",
            label: "etcd",
            default_port: 2379,
        },
        "zookeeper" => SchemeProfile {
            db_type: DatabaseType::ZooKeeper,
            profile: "zookeeper",
            label: "Apache ZooKeeper",
            default_port: 2181,
        },
        "mongodb" | "mongodb+srv" => SchemeProfile {
            db_type: DatabaseType::MongoDb,
            profile: "mongodb",
            label: "MongoDB",
            default_port: 27017,
        },
        "clickhouse" => SchemeProfile {
            db_type: DatabaseType::ClickHouse,
            profile: "clickhouse",
            label: "ClickHouse",
            default_port: 8123,
        },
        "sqlserver" | "mssql" => SchemeProfile {
            db_type: DatabaseType::SqlServer,
            profile: "sqlserver",
            label: "SQL Server",
            default_port: 1433,
        },
        "oracle" => SchemeProfile {
            db_type: DatabaseType::Oracle,
            profile: "oracle",
            label: "Oracle",
            default_port: 1521,
        },
        "elasticsearch" => SchemeProfile {
            db_type: DatabaseType::Elasticsearch,
            profile: "elasticsearch",
            label: "Elasticsearch",
            default_port: 9200,
        },
        "qdrant" => SchemeProfile {
            db_type: DatabaseType::Qdrant,
            profile: "qdrant",
            label: "Qdrant",
            default_port: 6333,
        },
        "milvus" => SchemeProfile {
            db_type: DatabaseType::Milvus,
            profile: "milvus",
            label: "Milvus",
            default_port: 19530,
        },
        "weaviate" => SchemeProfile {
            db_type: DatabaseType::Weaviate,
            profile: "weaviate",
            label: "Weaviate",
            default_port: 8080,
        },
        "chromadb" => SchemeProfile {
            db_type: DatabaseType::ChromaDb,
            profile: "chromadb",
            label: "ChromaDB",
            default_port: 8000,
        },
        "dm" | "dameng" => SchemeProfile {
            db_type: DatabaseType::Dameng,
            profile: "dm",
            label: "达梦 Dameng",
            default_port: 5236,
        },
        "kingbase" | "kingbase8" => SchemeProfile {
            db_type: DatabaseType::Kingbase,
            profile: "kingbase",
            label: "KingBase",
            default_port: 54321,
        },
        "gaussdb" => SchemeProfile {
            db_type: DatabaseType::Gaussdb,
            profile: "gaussdb",
            label: "GaussDB",
            default_port: 5432,
        },
        "kwdb" => SchemeProfile {
            db_type: DatabaseType::Kwdb,
            profile: "kwdb",
            label: "KWDB",
            default_port: 26257,
        },
        "gbase" => SchemeProfile {
            db_type: DatabaseType::Gbase,
            profile: "gbase",
            label: "GBase",
            default_port: 5258,
        },
        "gbasedbt-sqli" => SchemeProfile {
            db_type: DatabaseType::Gbase,
            profile: "gbase8s",
            label: "GBase 8s",
            default_port: 9088,
        },
        "informix-sqli" => SchemeProfile {
            db_type: DatabaseType::Informix,
            profile: "informix",
            label: "Informix",
            default_port: 9088,
        },
        "yashandb" => SchemeProfile {
            db_type: DatabaseType::Yashandb,
            profile: "yashandb",
            label: "YashanDB",
            default_port: 1688,
        },
        "opengauss" => SchemeProfile {
            db_type: DatabaseType::Gaussdb,
            profile: "opengauss",
            label: "openGauss",
            default_port: 5432,
        },
        "questdb" => SchemeProfile {
            db_type: DatabaseType::Questdb,
            profile: "questdb",
            label: "QuestDB",
            default_port: 8812,
        },
        "tdengine" | "taos-ws" => SchemeProfile {
            db_type: DatabaseType::Tdengine,
            profile: "tdengine",
            label: "TDengine",
            default_port: 6041,
        },
        "oscar" => SchemeProfile {
            db_type: DatabaseType::Oscar,
            profile: "oscar",
            label: "神通 OSCAR",
            default_port: 2003,
        },
        "xugu" => SchemeProfile {
            db_type: DatabaseType::Xugu,
            profile: "xugu",
            label: "XuguDB",
            default_port: 5138,
        },
        "iotdb" => SchemeProfile {
            db_type: DatabaseType::Iotdb,
            profile: "iotdb",
            label: "Apache IoTDB",
            default_port: 6667,
        },
        "iris" => SchemeProfile {
            db_type: DatabaseType::Iris,
            profile: "iris",
            label: "IRIS",
            default_port: 1972,
        },
        _ => return None,
    })
}

fn http_selected_profile(preferred: &str) -> Option<SchemeProfile> {
    match preferred {
        "clickhouse" | "elasticsearch" | "qdrant" | "milvus" | "weaviate" | "chromadb" => {
            scheme_profile(preferred)
        }
        _ => None,
    }
}

fn decode_url_part(value: &str) -> String {
    percent_decode_str(value).decode_utf8_lossy().into_owned()
}

fn database_from_path(pathname: &str) -> Option<String> {
    let value = pathname.trim_start_matches('/');
    if value.is_empty() {
        return None;
    }
    let first = value.split('/').next().unwrap_or("");
    if first.is_empty() {
        None
    } else {
        Some(decode_url_part(first))
    }
}

fn query_param_value(params: &str, key: &str) -> Option<String> {
    for part in params.split(['&', ';']) {
        if part.is_empty() {
            continue;
        }
        let mut iter = part.splitn(2, '=');
        let raw_key = iter.next().unwrap_or("");
        let value = iter.next().unwrap_or("");
        if decode_url_part(raw_key).eq_ignore_ascii_case(key) {
            let trimmed = decode_url_part(value).trim().to_string();
            return Some(trimmed);
        }
    }
    None
}

fn connection_name_param(params: &str) -> Option<String> {
    query_param_value(params, "name").filter(|v| !v.is_empty())
}

fn strip_connection_name_param(params: &str) -> String {
    if params.is_empty() {
        return String::new();
    }
    params
        .split('&')
        .filter(|part| {
            if part.is_empty() {
                return true;
            }
            let raw_key = part.split('=').next().unwrap_or("");
            !decode_url_part(raw_key).trim().eq_ignore_ascii_case("name")
        })
        .collect::<Vec<_>>()
        .join("&")
}

fn url_params_require_tls(db_type: DatabaseType, params: &str) -> bool {
    match db_type {
        DatabaseType::Mysql => {
            let require_ssl = query_param_value(params, "require_ssl")
                .unwrap_or_default()
                .to_ascii_lowercase();
            if matches!(require_ssl.as_str(), "true" | "1" | "yes") {
                return true;
            }
            let ssl_mode = query_param_value(params, "ssl-mode")
                .or_else(|| query_param_value(params, "sslmode"))
                .unwrap_or_default()
                .to_ascii_lowercase()
                .replace('-', "_");
            matches!(
                ssl_mode.as_str(),
                "required" | "require" | "verify_ca" | "verify_identity"
            )
        }
        DatabaseType::Postgres | DatabaseType::Redshift | DatabaseType::Kwdb => {
            let ssl_mode = query_param_value(params, "sslmode")
                .unwrap_or_default()
                .to_ascii_lowercase();
            matches!(ssl_mode.as_str(), "require" | "verify-ca" | "verify-full")
        }
        _ => false,
    }
}

fn is_tidb_cloud_host(host: &str) -> bool {
    host.to_ascii_lowercase().ends_with(".tidbcloud.com")
}

fn parse_host_port(authority_host: &str, default_port: u16) -> (String, u16, bool) {
    let host_part = authority_host.trim();
    if host_part.starts_with('[') {
        if let Some(end) = host_part.find(']') {
            let host = host_part[1..end].to_string();
            let rest = &host_part[end + 1..];
            if let Some(port_str) = rest.strip_prefix(':') {
                if let Ok(port) = port_str.parse::<u16>() {
                    return (host, port, true);
                }
            }
            return (host, default_port, false);
        }
    }
    if let Some((host, port_str)) = host_part.rsplit_once(':') {
        if !host.contains(':') {
            if let Ok(port) = port_str.parse::<u16>() {
                return (host.to_string(), port, true);
            }
        }
    }
    (host_part.to_string(), default_port, false)
}

fn parse_userinfo(userinfo: Option<&str>) -> (String, String) {
    match userinfo {
        Some(info) if !info.is_empty() => match info.split_once(':') {
            Some((user, pass)) => (decode_url_part(user), decode_url_part(pass)),
            None => (decode_url_part(info), String::new()),
        },
        _ => (String::new(), String::new()),
    }
}

/// Manual authority parse: `[userinfo@]host[:port][/path][?query][#fragment]`
struct AuthorityParts {
    username: String,
    password: String,
    host: String,
    port: u16,
    port_explicit: bool,
    path: String,
    query: String,
    fragment: String,
}

fn split_url_parts(source: &str) -> Result<(String, AuthorityParts), String> {
    let (scheme, rest) = source
        .split_once("://")
        .ok_or_else(|| "Invalid connection URL: missing scheme://".to_string())?;
    let scheme = scheme.to_ascii_lowercase();

    let (before_hash, fragment) = match rest.split_once('#') {
        Some((b, f)) => (b, f.to_string()),
        None => (rest, String::new()),
    };
    let (before_query, query) = match before_hash.split_once('?') {
        Some((b, q)) => (b, q.to_string()),
        None => (before_hash, String::new()),
    };

    // Last '@' separates userinfo from host (so passwords may contain '@' when encoded, or
    // when only one host '@' remains). Prefer encoding password '@' as %40 for portability.
    let (userinfo, hostport_path) = match before_query.rsplit_once('@') {
        Some((userinfo, hostport_path)) => (Some(userinfo), hostport_path),
        None => (None, before_query),
    };

    let (hostport, path) = if let Some(slash) = hostport_path.find('/') {
        (&hostport_path[..slash], hostport_path[slash..].to_string())
    } else {
        (hostport_path, String::new())
    };

    if hostport.trim().is_empty() {
        return Err(
            "Invalid connection URL: empty host (if the password contains '@', encode it as %40)"
                .into(),
        );
    }

    let (username, password) = parse_userinfo(userinfo);
    // Port default filled later by caller with profile default; use 0 as placeholder.
    let (host, port, port_explicit) = parse_host_port(hostport, 0);
    if host.trim().is_empty() {
        return Err(
            "Invalid connection URL: empty host (if the password contains '@', encode it as %40)"
                .into(),
        );
    }

    Ok((
        scheme,
        AuthorityParts {
            username,
            password,
            host,
            port,
            port_explicit,
            path,
            query,
            fragment,
        },
    ))
}

fn normalize_mongo_connection_string(value: &str) -> String {
    let input = value.trim();
    let Some(caps) = regex_lite_mongo_prefix(input) else {
        return input.to_string();
    };
    let (prefix, userinfo) = caps;
    let Some(userinfo) = userinfo else {
        return input.to_string();
    };
    let (username, password) = match userinfo.split_once(':') {
        Some((u, p)) => (u, Some(p)),
        None => (userinfo, None),
    };
    let encoded_user = encode_mongo_userinfo_part(username);
    let encoded_pass = password
        .map(|p| format!(":{}", encode_mongo_userinfo_part(p)))
        .unwrap_or_default();
    // Replace only the userinfo segment after scheme://
    if let Some(at) = input.find('@') {
        format!("{prefix}{encoded_user}{encoded_pass}@{}", &input[at + 1..])
    } else {
        input.to_string()
    }
}

fn regex_lite_mongo_prefix(input: &str) -> Option<(String, Option<&str>)> {
    let lower = input.to_ascii_lowercase();
    let prefix_len = if lower.starts_with("mongodb+srv://") {
        "mongodb+srv://".len()
    } else if lower.starts_with("mongodb://") {
        "mongodb://".len()
    } else {
        return None;
    };
    let prefix = &input[..prefix_len];
    let rest = &input[prefix_len..];
    let userinfo = rest.split_once('@').map(|(u, _)| u);
    Some((prefix.to_string(), userinfo))
}

fn encode_mongo_userinfo_part(value: &str) -> String {
    let decoded = percent_decode_str(value).decode_utf8_lossy();
    percent_encoding::utf8_percent_encode(&decoded, percent_encoding::NON_ALPHANUMERIC).to_string()
}

fn parse_mongo_url(source: &str) -> Option<ParsedConnectionUrl> {
    let (scheme_raw, rest) = source.split_once("://")?;
    let scheme = scheme_raw.to_ascii_lowercase();
    if scheme != "mongodb" && scheme != "mongodb+srv" {
        return None;
    }
    let profile = scheme_profile(&scheme)?;

    let (before_hash, _fragment) = match rest.split_once('#') {
        Some((b, f)) => (b, f),
        None => (rest, ""),
    };
    let (before_query, query) = match before_hash.split_once('?') {
        Some((b, q)) => (b, q.to_string()),
        None => (before_hash, String::new()),
    };
    let (userinfo, hosts_path) = match before_query.rsplit_once('@') {
        Some((u, h)) => (Some(u), h),
        None => (None, before_query),
    };
    let (hosts, path) = match hosts_path.find('/') {
        Some(idx) => (&hosts_path[..idx], hosts_path[idx..].to_string()),
        None => (hosts_path, String::new()),
    };
    let first_host = hosts.split(',').next().unwrap_or(hosts);
    let (host, port, port_explicit) = parse_host_port(first_host, profile.default_port);
    let (username, password) = parse_userinfo(userinfo);

    Some(ParsedConnectionUrl {
        name: connection_name_param(&query),
        db_type: profile.db_type,
        driver_profile: profile.profile.to_string(),
        driver_label: profile.label.to_string(),
        host,
        port,
        username,
        password,
        database: database_from_path(&path),
        url_params: query,
        ssl: scheme == "mongodb+srv",
        connection_string: Some(normalize_mongo_connection_string(source)),
        oracle_connection_type: None,
        use_mongo_url: true,
        port_explicit,
    })
}

fn parse_jdbc_sqlserver_url(source: &str) -> Option<ParsedConnectionUrl> {
    let lower = source.to_ascii_lowercase();
    if !lower.starts_with("jdbc:sqlserver://") {
        return None;
    }
    let rest = &source["jdbc:sqlserver://".len()..];
    let (hostport, props_raw) = match rest.split_once(';') {
        Some((h, p)) => (h, p),
        None => (rest, ""),
    };
    let profile = scheme_profile("sqlserver")?;
    let (host, port, port_explicit) = parse_host_port(hostport, profile.default_port);
    let mut database = None;
    let mut username = String::new();
    let mut password = String::new();
    let mut url_params = Vec::new();
    for part in props_raw.split(';') {
        if part.is_empty() {
            continue;
        }
        let mut iter = part.splitn(2, '=');
        let key = iter.next().unwrap_or("").trim();
        let value = iter.next().unwrap_or("");
        match key.to_ascii_lowercase().as_str() {
            "databasename" | "database" => database = Some(decode_url_part(value)).filter(|v| !v.is_empty()),
            "user" => username = decode_url_part(value),
            "password" => password = decode_url_part(value),
            _ => url_params.push(part.to_string()),
        }
    }
    Some(ParsedConnectionUrl {
        name: None,
        db_type: profile.db_type,
        driver_profile: profile.profile.to_string(),
        driver_label: profile.label.to_string(),
        host,
        port,
        username,
        password,
        database,
        url_params: url_params.join(";"),
        ssl: false,
        connection_string: None,
        oracle_connection_type: None,
        use_mongo_url: false,
        port_explicit,
    })
}

fn oracle_descriptor_value(source: &str, key: &str) -> Option<String> {
    let pattern = format!("({key}");
    let upper = source.to_ascii_uppercase();
    let key_upper = format!("({}", key.to_ascii_uppercase());
    let idx = upper.find(&key_upper)?;
    let after = &source[idx + pattern.len()..];
    let after = after.trim_start();
    let after = after.strip_prefix('=')?.trim_start();
    let end = after.find(')')?;
    let value = after[..end].trim();
    if value.is_empty() {
        None
    } else {
        Some(value.to_string())
    }
}

fn parse_jdbc_oracle_url(source: &str) -> Option<ParsedConnectionUrl> {
    let lower = source.to_ascii_lowercase();
    if !lower.starts_with("jdbc:oracle:thin:@") {
        return None;
    }
    let profile = scheme_profile("oracle")?;
    let after = &source["jdbc:oracle:thin:@".len()..];
    let after_trim = after.trim_start();

    if after_trim.starts_with('(') {
        let host = oracle_descriptor_value(source, "HOST")?;
        let port = oracle_descriptor_value(source, "PORT")
            .and_then(|p| p.parse().ok())
            .unwrap_or(profile.default_port);
        let service_name = oracle_descriptor_value(source, "SERVICE_NAME");
        let sid = oracle_descriptor_value(source, "SID");
        let oracle_type = if sid.is_some() && service_name.is_none() {
            "sid"
        } else {
            "service_name"
        };
        return Some(ParsedConnectionUrl {
            name: None,
            db_type: profile.db_type,
            driver_profile: profile.profile.to_string(),
            driver_label: profile.label.to_string(),
            host,
            port,
            username: String::new(),
            password: String::new(),
            database: service_name.or(sid),
            url_params: String::new(),
            ssl: false,
            connection_string: Some(source.to_string()),
            oracle_connection_type: Some(oracle_type.to_string()),
            use_mongo_url: false,
            port_explicit: false,
        });
    }

    // //@host:port/service
    if let Some(rest) = after_trim.strip_prefix("//") {
        let (hostport_db, query) = match rest.split_once('?') {
            Some((h, q)) => (h, q),
            None => (rest, ""),
        };
        let (hostport, database) = hostport_db.split_once('/')?;
        let (host, port, port_explicit) = parse_host_port(hostport, profile.default_port);
        return Some(ParsedConnectionUrl {
            name: None,
            db_type: profile.db_type,
            driver_profile: profile.profile.to_string(),
            driver_label: profile.label.to_string(),
            host,
            port,
            username: String::new(),
            password: String::new(),
            database: Some(decode_url_part(database)).filter(|v| !v.is_empty()),
            url_params: query.to_string(),
            ssl: false,
            connection_string: None,
            oracle_connection_type: Some("service_name".to_string()),
            use_mongo_url: false,
            port_explicit,
        });
    }

    // host:port:sid
    let (hostport_sid, query) = match after_trim.split_once('?') {
        Some((h, q)) => (h, q),
        None => (after_trim, ""),
    };
    let mut parts = hostport_sid.splitn(3, ':');
    let host = parts.next()?.to_string();
    let port_or_sid = parts.next()?;
    if let Some(sid) = parts.next() {
        let port = port_or_sid.parse().unwrap_or(profile.default_port);
        return Some(ParsedConnectionUrl {
            name: None,
            db_type: profile.db_type,
            driver_profile: profile.profile.to_string(),
            driver_label: profile.label.to_string(),
            host,
            port,
            username: String::new(),
            password: String::new(),
            database: Some(decode_url_part(sid)).filter(|v| !v.is_empty()),
            url_params: query.to_string(),
            ssl: false,
            connection_string: None,
            oracle_connection_type: Some("sid".to_string()),
            use_mongo_url: false,
            port_explicit: true,
        });
    }
    None
}

/// Parse a database connection URL (also accepts `connection_url` / `dsn` synonyms at call sites).
///
/// Supported schemes match the desktop Connection Dialog parser. Types without a conventional
/// URL scheme (e.g. SQLite file paths, DuckDB, BigQuery, Snowflake key-pair, MQ) are not
/// accepted here — use explicit JSON fields instead.
pub fn parse_connection_url(value: &str) -> Result<ParsedConnectionUrl, String> {
    parse_connection_url_with_profile(value, None)
}

/// Like [`parse_connection_url`], but `http`/`https` can resolve via `preferred_profile`
/// (clickhouse / elasticsearch / qdrant / milvus / weaviate / chromadb).
pub fn parse_connection_url_with_profile(
    value: &str,
    preferred_profile: Option<&str>,
) -> Result<ParsedConnectionUrl, String> {
    let input = value.trim();
    if input.is_empty() {
        return Err("Connection URL is empty".into());
    }

    if let Some(parsed) = parse_jdbc_sqlserver_url(input) {
        return Ok(parsed);
    }
    if let Some(parsed) = parse_jdbc_oracle_url(input) {
        return Ok(parsed);
    }

    let is_jdbc = input.to_ascii_lowercase().starts_with("jdbc:");
    let source = if is_jdbc {
        input.get(5..).unwrap_or(input)
    } else {
        input
    };

    if let Some(parsed) = parse_mongo_url(source) {
        return Ok(parsed);
    }

    let (scheme, mut parts) = match split_url_parts(source) {
        Ok(v) => v,
        Err(err) => return Err(with_userinfo_encoding_hint(source, err)),
    };
    let profile = if matches!(scheme.as_str(), "http" | "https") {
        preferred_profile
            .and_then(http_selected_profile)
            .ok_or_else(|| {
                format!(
                    "Unsupported connection URL scheme: {scheme} (provide type/driver_profile for http/https)"
                )
            })?
    } else {
        scheme_profile(&scheme).ok_or_else(|| {
            with_userinfo_encoding_hint(
                source,
                format!("Unsupported connection URL scheme: {scheme}"),
            )
        })?
    };

    if parts.port == 0 {
        parts.port = profile.default_port;
        parts.port_explicit = false;
    }

    let name = connection_name_param(&parts.query);
    let url_params_without_name = strip_connection_name_param(&parts.query);
    let normalized_fragment = decode_url_part(&parts.fragment).trim().to_ascii_lowercase();
    let parsed_url_params = if profile.db_type == DatabaseType::Redis && normalized_fragment == "insecure"
    {
        if url_params_without_name.is_empty() {
            "insecure=true".to_string()
        } else {
            format!("{url_params_without_name}&insecure=true")
        }
    } else {
        url_params_without_name
    };

    if profile.db_type == DatabaseType::MongoDb {
        return Ok(ParsedConnectionUrl {
            name,
            db_type: profile.db_type,
            driver_profile: profile.profile.to_string(),
            driver_label: profile.label.to_string(),
            host: parts.host,
            port: parts.port,
            username: parts.username,
            password: parts.password,
            database: database_from_path(&parts.path),
            url_params: parsed_url_params,
            ssl: scheme == "mongodb+srv",
            connection_string: Some(normalize_mongo_connection_string(source)),
            oracle_connection_type: None,
            use_mongo_url: true,
            port_explicit: parts.port_explicit,
        });
    }

    if profile.db_type == DatabaseType::ZooKeeper {
        let host_for_cs = if parts.host.contains(':') {
            format!("[{}]", parts.host)
        } else {
            parts.host.clone()
        };
        let chroot = if parts.path.is_empty() || parts.path == "/" {
            String::new()
        } else {
            parts.path.clone()
        };
        let connection_string = format!("{}:{}{}", host_for_cs, parts.port, chroot);
        return Ok(ParsedConnectionUrl {
            name,
            db_type: profile.db_type,
            driver_profile: profile.profile.to_string(),
            driver_label: profile.label.to_string(),
            host: parts.host,
            port: parts.port,
            username: parts.username,
            password: parts.password,
            database: None,
            url_params: strip_connection_name_param(&parts.query),
            ssl: false,
            connection_string: Some(connection_string),
            oracle_connection_type: None,
            use_mongo_url: false,
            port_explicit: parts.port_explicit,
        });
    }

    let ssl = scheme == "rediss"
        || scheme == "https"
        || url_params_require_tls(profile.db_type, &parsed_url_params)
        || (profile.db_type == DatabaseType::Mysql && is_tidb_cloud_host(&parts.host));

    Ok(ParsedConnectionUrl {
        name,
        db_type: profile.db_type,
        driver_profile: profile.profile.to_string(),
        driver_label: profile.label.to_string(),
        host: parts.host,
        port: parts.port,
        username: parts.username,
        password: parts.password,
        database: database_from_path(&parts.path),
        url_params: parsed_url_params,
        ssl,
        connection_string: None,
        oracle_connection_type: None,
        use_mongo_url: false,
        port_explicit: parts.port_explicit && profile.db_type == DatabaseType::SqlServer,
    })
}

fn with_userinfo_encoding_hint(source: &str, err: String) -> String {
    if source.contains('@') && !err.contains("%40") {
        format!(
            "{err}. If the password contains '@' or other reserved characters, URL-encode them (e.g. '@' -> %40)."
        )
    } else {
        err
    }
}

/// Human-readable list of schemes accepted by [`parse_connection_url`].
pub fn supported_connection_url_schemes() -> &'static [&'static str] {
    &[
        "mysql",
        "mariadb",
        "postgres",
        "postgresql",
        "redshift",
        "redis",
        "rediss",
        "etcd",
        "zookeeper",
        "mongodb",
        "mongodb+srv",
        "clickhouse",
        "sqlserver",
        "mssql",
        "oracle",
        "elasticsearch",
        "qdrant",
        "milvus",
        "weaviate",
        "chromadb",
        "dm",
        "dameng",
        "kingbase",
        "kingbase8",
        "gaussdb",
        "kwdb",
        "gbase",
        "gbasedbt-sqli",
        "informix-sqli",
        "yashandb",
        "opengauss",
        "questdb",
        "tdengine",
        "taos-ws",
        "oscar",
        "xugu",
        "iotdb",
        "iris",
        "jdbc:mysql",
        "jdbc:mariadb",
        "jdbc:postgresql",
        "jdbc:sqlserver",
        "jdbc:oracle:thin",
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_mysql_url() {
        let parsed = parse_connection_url("mysql://root:secret@127.0.0.1:3306/app?ssl-mode=required").unwrap();
        assert_eq!(parsed.db_type, DatabaseType::Mysql);
        assert_eq!(parsed.host, "127.0.0.1");
        assert_eq!(parsed.port, 3306);
        assert_eq!(parsed.username, "root");
        assert_eq!(parsed.password, "secret");
        assert_eq!(parsed.database.as_deref(), Some("app"));
        assert!(parsed.ssl);
        assert_eq!(parsed.driver_profile, "mysql");
    }

    #[test]
    fn parses_postgres_url_with_name_param() {
        let parsed =
            parse_connection_url("postgres://u:p@db.example:5433/mydb?sslmode=require&name=prod-pg").unwrap();
        assert_eq!(parsed.db_type, DatabaseType::Postgres);
        assert_eq!(parsed.port, 5433);
        assert_eq!(parsed.name.as_deref(), Some("prod-pg"));
        assert!(parsed.ssl);
        assert!(!parsed.url_params.contains("name="));
        assert!(parsed.url_params.contains("sslmode=require"));
    }

    #[test]
    fn parses_mongodb_srv() {
        let parsed = parse_connection_url("mongodb+srv://user:pass@cluster.example/app").unwrap();
        assert_eq!(parsed.db_type, DatabaseType::MongoDb);
        assert!(parsed.ssl);
        assert!(parsed.use_mongo_url);
        assert!(parsed.connection_string.is_some());
    }

    #[test]
    fn rejects_unknown_scheme() {
        let err = parse_connection_url("foo://bar").unwrap_err();
        assert!(err.contains("Unsupported"));
    }

    #[test]
    fn parses_jdbc_mysql_and_postgresql_prefix() {
        let mysql = parse_connection_url("jdbc:mysql://root:secret@127.0.0.1:3306/app").unwrap();
        assert_eq!(mysql.db_type, DatabaseType::Mysql);
        assert_eq!(mysql.host, "127.0.0.1");
        assert_eq!(mysql.database.as_deref(), Some("app"));

        let pg = parse_connection_url("jdbc:postgresql://u:p@db.example:5432/warehouse").unwrap();
        assert_eq!(pg.db_type, DatabaseType::Postgres);
        assert_eq!(pg.host, "db.example");
        assert_eq!(pg.database.as_deref(), Some("warehouse"));

        let maria = parse_connection_url("jdbc:mariadb://u:p@h:3306/db").unwrap();
        assert_eq!(maria.driver_profile, "mariadb");
    }

    #[test]
    fn parses_password_with_at_via_last_separator() {
        let parsed = parse_connection_url("mysql://user:aD5@cC02@10.0.0.1:3306/app").unwrap();
        assert_eq!(parsed.username, "user");
        assert_eq!(parsed.password, "aD5@cC02");
        assert_eq!(parsed.host, "10.0.0.1");
    }

    #[test]
    fn parses_password_with_encoded_at() {
        let parsed = parse_connection_url("mysql://user:aD5%40cC02@10.0.0.1:3306/app").unwrap();
        assert_eq!(parsed.password, "aD5@cC02");
        assert_eq!(parsed.host, "10.0.0.1");
    }

    #[test]
    fn parses_jdbc_sqlserver() {
        let parsed =
            parse_connection_url("jdbc:sqlserver://dbhost:1433;databaseName=app;user=sa;password=x").unwrap();
        assert_eq!(parsed.db_type, DatabaseType::SqlServer);
        assert_eq!(parsed.host, "dbhost");
        assert_eq!(parsed.database.as_deref(), Some("app"));
        assert_eq!(parsed.username, "sa");
    }
}
