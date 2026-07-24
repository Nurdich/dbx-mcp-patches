//! Connection progress logging (CLI stderr stream / MCP buffered prepend).

use std::cell::RefCell;
use std::sync::{Arc, Mutex};

use dbx_core::models::connection::{ConnectionConfig, TransportLayerConfig};

thread_local! {
    static ACTIVE: RefCell<Option<ProgressOptions>> = const { RefCell::new(None) };
}

#[derive(Clone, Default)]
pub struct ProgressOptions {
    pub quiet: bool,
    pub verbose: bool,
    /// When set, messages are appended here (MCP). When None, CLI writes stderr.
    pub collector: Option<Arc<Mutex<Vec<String>>>>,
}

pub struct ProgressGuard;

impl Drop for ProgressGuard {
    fn drop(&mut self) {
        ACTIVE.with(|slot| *slot.borrow_mut() = None);
        dbx_core::connect_progress::set_hook(None);
    }
}

pub fn push_progress(options: ProgressOptions) -> ProgressGuard {
    ACTIVE.with(|slot| *slot.borrow_mut() = Some(options));
    // Mirror core tunnel/failover emits into the same [dbx] stream.
    dbx_core::connect_progress::set_hook(Some(Arc::new(|message: &str| {
        connection_log(message, false);
    })));
    ProgressGuard
}

pub fn mcp_progress_options() -> ProgressOptions {
    let quiet = env_truthy("DBX_MCP_QUIET");
    let verbose = env_truthy("DBX_MCP_VERBOSE");
    ProgressOptions {
        quiet,
        verbose,
        collector: Some(Arc::new(Mutex::new(Vec::new()))),
    }
}

pub fn cli_progress_options(quiet: bool, verbose: bool) -> ProgressOptions {
    ProgressOptions { quiet, verbose, collector: None }
}

fn env_truthy(name: &str) -> bool {
    std::env::var(name).ok().is_some_and(|value| {
        matches!(value.trim().to_ascii_lowercase().as_str(), "1" | "true" | "yes")
    })
}

pub fn connection_log(message: impl AsRef<str>, verbose_only: bool) {
    ACTIVE.with(|slot| {
        let Some(options) = slot.borrow().clone() else {
            return;
        };
        if options.quiet {
            return;
        }
        if verbose_only && !options.verbose {
            return;
        }
        let line = format!("[dbx] {}\n", message.as_ref());
        if let Some(collector) = &options.collector {
            if let Ok(mut logs) = collector.lock() {
                logs.push(line);
            }
        } else {
            eprint!("{line}");
        }
    });
}

pub fn log_query_sql(sql: &str) {
    let normalized = sql.split_whitespace().collect::<Vec<_>>().join(" ");
    connection_log(format!("SQL: {normalized}"), true);
}

pub fn describe_connection_target(config: &ConnectionConfig) -> String {
    let db_type = serde_json::to_value(config.db_type)
        .ok()
        .and_then(|value| value.as_str().map(ToOwned::to_owned))
        .unwrap_or_else(|| format!("{:?}", config.db_type).to_ascii_lowercase());
    let target = format!("{db_type} @ {}:{}", config.host, config.port);
    match config.database.as_deref().map(str::trim).filter(|v| !v.is_empty()) {
        Some(database) => format!("{target}/{database}"),
        None => target,
    }
}

pub fn log_using_connection(config: &ConnectionConfig) {
    connection_log(
        format!("Using connection \"{}\" ({})", config.name, describe_connection_target(config)),
        false,
    );
    let proxy_layers: Vec<_> = config
        .transport_layers
        .iter()
        .filter_map(|layer| match layer {
            TransportLayerConfig::Proxy(proxy) => Some(proxy),
            _ => None,
        })
        .collect();
    let failover = proxy_layers.len() >= 2 && proxy_layers.iter().skip(1).all(|proxy| !proxy.enabled);
    if failover {
        let labels: Vec<String> = proxy_layers
            .iter()
            .enumerate()
            .map(|(index, proxy)| {
                let name = if !proxy.name.is_empty() {
                    proxy.name.as_str()
                } else if !proxy.profile_id.is_empty() {
                    proxy.profile_id.as_str()
                } else {
                    proxy.id.as_str()
                };
                format!("#{} ({name})", index + 1)
            })
            .collect();
        connection_log(
            format!(
                "Proxy failover candidates: {} (try next on failure, not multi-hop chained)",
                labels.join(", ")
            ),
            false,
        );
    }
    for layer in &config.transport_layers {
        if !layer.enabled() {
            continue;
        }
        match layer {
            TransportLayerConfig::Proxy(proxy) => {
                if !proxy.profile_id.is_empty() {
                    connection_log(
                        format!(
                            "Using saved proxy profile: {}",
                            if proxy.name.is_empty() {
                                &proxy.profile_id
                            } else {
                                &proxy.name
                            }
                        ),
                        false,
                    );
                } else {
                    let kind = match proxy.proxy_type {
                        dbx_core::models::connection::ProxyType::Http => "http",
                        dbx_core::models::connection::ProxyType::Socks5 => "socks5",
                    };
                    connection_log(
                        format!(
                            "Connecting via {kind} proxy {}:{}",
                            proxy.host,
                            if proxy.port == 0 { 1080 } else { proxy.port }
                        ),
                        false,
                    );
                }
            }
            TransportLayerConfig::Ssh(ssh) => {
                let label = if !ssh.name.is_empty() {
                    ssh.name.as_str()
                } else if !ssh.profile_id.is_empty() {
                    ssh.profile_id.as_str()
                } else {
                    ssh.id.as_str()
                };
                connection_log(
                    format!(
                        "SSH tunnel via {}@{}:{}{}",
                        ssh.user,
                        ssh.host,
                        if ssh.port == 0 { 22 } else { ssh.port },
                        if label.is_empty() {
                            String::new()
                        } else {
                            format!(" ({label})")
                        }
                    ),
                    false,
                );
            }
            TransportLayerConfig::HttpTunnel(_) => {}
        }
    }
}

pub fn collector_text(options: &ProgressOptions) -> String {
    options
        .collector
        .as_ref()
        .and_then(|collector| collector.lock().ok().map(|logs| logs.join("")))
        .unwrap_or_default()
}

pub fn format_progress_section(progress: &str) -> String {
    let trimmed = progress.trim_end();
    if trimmed.is_empty() {
        String::new()
    } else {
        format!("{trimmed}\n\n")
    }
}

pub async fn with_stage<T, F, Fut>(stage: &str, fut: F) -> Result<T, String>
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = Result<T, String>>,
{
    let label = if stage.ends_with("...") { stage.to_string() } else { format!("{stage}...") };
    connection_log(&label, false);
    match fut().await {
        Ok(value) => Ok(value),
        Err(error) => {
            let stage_name = stage.trim_end_matches('.');
            connection_log(format!("{stage_name} failed: {error}"), false);
            Err(format!("{stage_name}: {error}"))
        }
    }
}
