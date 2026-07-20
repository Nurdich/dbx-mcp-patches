//! Saved tunnel / proxy profile helpers (DBX Settings → Tunnels).

use dbx_core::models::connection::{ProxyTunnelConfig, ProxyType, TransportLayerConfig};
use uuid::Uuid;

use crate::list_index::{item_at_list_index, parse_list_index};

pub type TunnelProfile = TransportLayerConfig;

#[derive(Debug, Clone, Default)]
pub struct ProxyProfileRefArgs {
    pub proxy_profile_id: Option<String>,
    pub proxy_profile_name: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct InlineProxyArgs {
    pub proxy_enabled: Option<bool>,
    pub proxy_host: Option<String>,
    pub proxy_port: Option<u16>,
    pub proxy_username: Option<String>,
    pub proxy_password: Option<String>,
    pub proxy_type: Option<String>,
}

pub fn has_proxy_profile_ref(args: &ProxyProfileRefArgs) -> bool {
    args.proxy_profile_id.as_deref().is_some_and(|v| !v.trim().is_empty())
        || args.proxy_profile_name.as_deref().is_some_and(|v| !v.trim().is_empty())
}

pub fn has_inline_proxy_params(args: &InlineProxyArgs) -> bool {
    args.proxy_enabled == Some(true)
        || args.proxy_host.as_deref().is_some_and(|v| !v.trim().is_empty())
        || args.proxy_port.is_some()
        || args.proxy_username.as_deref().is_some_and(|v| !v.trim().is_empty())
        || args.proxy_password.as_ref().is_some_and(|v| !v.is_empty())
}

pub fn list_proxy_profiles(profiles: &[TunnelProfile]) -> Vec<&TunnelProfile> {
    profiles
        .iter()
        .filter(|profile| matches!(profile, TransportLayerConfig::Proxy(_)))
        .collect()
}

pub fn find_proxy_profiles_by_name<'a>(profiles: &'a [TunnelProfile], name: &str) -> Vec<&'a TunnelProfile> {
    let lower = name.trim().to_ascii_lowercase();
    list_proxy_profiles(profiles)
        .into_iter()
        .filter(|profile| profile.name().eq_ignore_ascii_case(&lower))
        .collect()
}

pub fn resolve_proxy_profile_by_index<'a>(profiles: &'a [TunnelProfile], index: usize) -> Option<&'a TunnelProfile> {
    let proxies = list_proxy_profiles(profiles);
    item_at_list_index(&proxies, index).copied()
}

pub fn find_proxy_profile<'a>(profiles: &'a [TunnelProfile], args: &ProxyProfileRefArgs) -> Option<&'a TunnelProfile> {
    if let Some(id) = args.proxy_profile_id.as_deref().map(str::trim).filter(|v| !v.is_empty()) {
        if let Some(match_by_id) = profiles.iter().find(|profile| {
            matches!(profile, TransportLayerConfig::Proxy(_)) && profile.id() == id
        }) {
            return Some(match_by_id);
        }
        if let Some(index) = parse_list_index(id) {
            return resolve_proxy_profile_by_index(profiles, index);
        }
        return None;
    }
    if let Some(name) = args.proxy_profile_name.as_deref().map(str::trim).filter(|v| !v.is_empty()) {
        let matches = find_proxy_profiles_by_name(profiles, name);
        if matches.len() == 1 {
            return Some(matches[0]);
        }
        if let Some(index) = parse_list_index(name) {
            return resolve_proxy_profile_by_index(profiles, index);
        }
    }
    None
}

pub fn proxy_profile_summary(profile: &TunnelProfile) -> String {
    match profile {
        TransportLayerConfig::Proxy(proxy) => {
            if proxy.host.is_empty() {
                if proxy.name.is_empty() {
                    proxy.id.clone()
                } else {
                    proxy.name.clone()
                }
            } else {
                let kind = match proxy.proxy_type {
                    ProxyType::Http => "http",
                    ProxyType::Socks5 => "socks5",
                };
                format!("{kind}://{}:{}", proxy.host, if proxy.port == 0 { 1080 } else { proxy.port })
            }
        }
        other => {
            if other.name().is_empty() {
                other.id().to_string()
            } else {
                other.name().to_string()
            }
        }
    }
}

pub fn proxy_profile_reference_layer(profile: &ProxyTunnelConfig, layer_id: String) -> TransportLayerConfig {
    TransportLayerConfig::Proxy(ProxyTunnelConfig {
        id: layer_id,
        name: profile.name.clone(),
        enabled: profile.enabled,
        proxy_type: profile.proxy_type,
        host: String::new(),
        port: 1080,
        username: String::new(),
        password: String::new(),
        profile_id: profile.id.clone(),
    })
}

pub fn apply_proxy_profile_override(
    mut config: dbx_core::models::connection::ConnectionConfig,
    profile: &ProxyTunnelConfig,
) -> dbx_core::models::connection::ConnectionConfig {
    let kept: Vec<_> = config
        .transport_layers
        .into_iter()
        .filter(|layer| !matches!(layer, TransportLayerConfig::Proxy(_)))
        .collect();
    let mut layers = kept;
    layers.push(proxy_profile_reference_layer(profile, Uuid::new_v4().to_string()));
    config.transport_layers = layers;
    config
}

pub fn inline_proxy_layer(args: &InlineProxyArgs) -> Result<TransportLayerConfig, String> {
    let host = args
        .proxy_host
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .ok_or_else(|| "proxy_host is required when configuring an inline proxy.".to_string())?
        .to_string();
    let port = args.proxy_port.unwrap_or(1080);
    let proxy_type = match args.proxy_type.as_deref().unwrap_or("socks5").trim().to_ascii_lowercase().as_str() {
        "http" => ProxyType::Http,
        "socks5" | "socks" | "" => ProxyType::Socks5,
        other => return Err(format!("Unsupported proxy_type \"{other}\". Use socks5 or http.")),
    };
    Ok(TransportLayerConfig::Proxy(ProxyTunnelConfig {
        id: Uuid::new_v4().to_string(),
        name: format!("{host}:{port}"),
        enabled: args.proxy_enabled.unwrap_or(true),
        proxy_type,
        host,
        port,
        username: args.proxy_username.clone().unwrap_or_default(),
        password: args.proxy_password.clone().unwrap_or_default(),
        profile_id: String::new(),
    }))
}

pub fn format_proxy_list(profiles: &[TunnelProfile]) -> String {
    let proxies = list_proxy_profiles(profiles);
    if proxies.is_empty() {
        return "No proxy tunnel profiles configured in DBX Settings > Tunnels.".to_string();
    }
    let mut output =
        String::from("| # | ID | Name | Endpoint | Enabled |\n| --- | --- | --- | --- | --- |");
    for (idx, profile) in proxies.iter().enumerate() {
        let TransportLayerConfig::Proxy(proxy) = profile else {
            continue;
        };
        output.push_str(&format!(
            "\n| {} | {} | {} | {} | {} |",
            idx + 1,
            escape_cell(&proxy.id),
            escape_cell(&proxy.name),
            escape_cell(&proxy_profile_summary(profile)),
            if proxy.enabled { "yes" } else { "no" },
        ));
    }
    output
}

fn escape_cell(value: &str) -> String {
    value.replace('|', "\\|").replace('\n', " ")
}
