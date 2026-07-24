//! Saved tunnel / proxy profile helpers (DBX Settings → Tunnels).
//!
//! Multi-proxy semantics here are **failover** (try next on failure), never
//! multi-hop chaining. Upstream `transport_layers` chaining is preserved for
//! mixed SSH/HTTP stacks; a failover group is stored as ordered Proxy stubs
//! where only the first is `enabled=true` and the rest are disabled candidates.

use crate::models::connection::{ProxyTunnelConfig, ProxyType, TransportLayerConfig};
use uuid::Uuid;

use crate::list_index::{item_at_list_index, parse_list_index, parse_list_index_range};

pub type TunnelProfile = TransportLayerConfig;

#[derive(Debug, Clone, Default)]
pub struct ProxyProfileRefArgs {
    /// Single id/index, comma list (`1,2,3`), range (`#1-#3`), or UUID.
    pub proxy_profile_id: Option<String>,
    /// Single name/index (legacy). Prefer `proxy_profile_names` for multiples.
    pub proxy_profile_name: Option<String>,
    /// Explicit ordered id/index list (MCP array). Merged with parsed `proxy_profile_id`.
    pub proxy_profile_ids: Option<Vec<String>>,
    /// Explicit ordered names (repeated CLI flags / MCP array).
    pub proxy_profile_names: Option<Vec<String>>,
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
        || args.proxy_profile_ids.as_ref().is_some_and(|v| v.iter().any(|s| !s.trim().is_empty()))
        || args.proxy_profile_names.as_ref().is_some_and(|v| v.iter().any(|s| !s.trim().is_empty()))
}

pub fn has_inline_proxy_params(args: &InlineProxyArgs) -> bool {
    args.proxy_enabled == Some(true)
        || args.proxy_host.as_deref().is_some_and(|v| !v.trim().is_empty())
        || args.proxy_port.is_some()
        || args.proxy_username.as_deref().is_some_and(|v| !v.trim().is_empty())
        || args.proxy_password.as_ref().is_some_and(|v| !v.is_empty())
}

/// Split comma-separated tokens and expand numeric ranges (`1-3`, `#1:#3`).
pub fn parse_proxy_ref_tokens(value: &str) -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    for part in value.split(',') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        match parse_list_index_range(part) {
            Ok(Some(indexes)) => {
                for index in indexes {
                    out.push(index.to_string());
                }
            }
            Ok(None) => out.push(part.to_string()),
            Err(error) => return Err(error.message),
        }
    }
    Ok(out)
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
    resolve_proxy_profiles(profiles, args).ok().and_then(|list| list.into_iter().next())
}

/// Resolve an ordered failover list of proxy profiles from id/name tokens.
pub fn resolve_proxy_profiles<'a>(
    profiles: &'a [TunnelProfile],
    args: &ProxyProfileRefArgs,
) -> Result<Vec<&'a TunnelProfile>, String> {
    let mut tokens: Vec<String> = Vec::new();

    if let Some(ids) = &args.proxy_profile_ids {
        for id in ids {
            let trimmed = id.trim();
            if !trimmed.is_empty() {
                tokens.extend(parse_proxy_ref_tokens(trimmed)?);
            }
        }
    }
    if let Some(id) = args.proxy_profile_id.as_deref().map(str::trim).filter(|v| !v.is_empty()) {
        tokens.extend(parse_proxy_ref_tokens(id)?);
    }
    if let Some(names) = &args.proxy_profile_names {
        for name in names {
            let trimmed = name.trim();
            if !trimmed.is_empty() {
                tokens.push(trimmed.to_string());
            }
        }
    }
    if let Some(name) = args.proxy_profile_name.as_deref().map(str::trim).filter(|v| !v.is_empty()) {
        // Allow comma lists in the legacy single field too.
        if name.contains(',') || parse_list_index_range(name).ok().flatten().is_some() {
            tokens.extend(parse_proxy_ref_tokens(name)?);
        } else {
            tokens.push(name.to_string());
        }
    }

    if tokens.is_empty() {
        return Ok(Vec::new());
    }

    let mut resolved = Vec::with_capacity(tokens.len());
    for token in tokens {
        let profile = resolve_one_proxy_token(profiles, &token)?;
        resolved.push(profile);
    }
    Ok(resolved)
}

fn resolve_one_proxy_token<'a>(profiles: &'a [TunnelProfile], token: &str) -> Result<&'a TunnelProfile, String> {
    let token = token.trim();
    if token.is_empty() {
        return Err("Empty proxy profile reference.".to_string());
    }

    if let Some(match_by_id) = profiles.iter().find(|profile| {
        matches!(profile, TransportLayerConfig::Proxy(_)) && profile.id() == token
    }) {
        return Ok(match_by_id);
    }

    if let Some(index) = parse_list_index(token) {
        return resolve_proxy_profile_by_index(profiles, index)
            .ok_or_else(|| format!("Proxy list index #{index} is out of range."));
    }

    let matches = find_proxy_profiles_by_name(profiles, token);
    match matches.as_slice() {
        [] => Err(format!("Proxy profile \"{token}\" not found.")),
        [profile] => Ok(*profile),
        many => {
            let lines: Vec<_> = many.iter().map(|profile| format!("- {} ({})", profile.id(), profile.name())).collect();
            Err(format!(
                "Multiple proxy profiles named \"{token}\". Specify proxy_profile_id or list index (#):\n{}",
                lines.join("\n")
            ))
        }
    }
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
        test_target: None,
        profile_id: profile.id.clone(),
    })
}

pub fn apply_proxy_profile_override(
    config: crate::models::connection::ConnectionConfig,
    profile: &ProxyTunnelConfig,
) -> crate::models::connection::ConnectionConfig {
    apply_proxy_profiles_failover(config, &[profile])
}

/// Replace existing proxy layers with an ordered **failover** group.
///
/// Storage: first profile stub `enabled=true`; subsequent stubs `enabled=false`.
/// Runtime (`dbx-core`) tries them in order. This is NOT multi-hop chaining —
/// only one proxy is active per attempt. Non-proxy layers (e.g. SSH) are kept.
pub fn apply_proxy_profiles_failover(
    mut config: crate::models::connection::ConnectionConfig,
    profiles: &[&ProxyTunnelConfig],
) -> crate::models::connection::ConnectionConfig {
    let kept: Vec<_> = config
        .transport_layers
        .into_iter()
        .filter(|layer| !matches!(layer, TransportLayerConfig::Proxy(_)))
        .collect();
    let mut layers = kept;
    for (index, profile) in profiles.iter().enumerate() {
        let mut layer = proxy_profile_reference_layer(profile, Uuid::new_v4().to_string());
        if let TransportLayerConfig::Proxy(ref mut proxy) = layer {
            proxy.enabled = index == 0;
        }
        layers.push(layer);
    }
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
        test_target: None,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_comma_and_range_tokens() {
        assert_eq!(parse_proxy_ref_tokens("1,2,3").unwrap(), vec!["1", "2", "3"]);
        assert_eq!(parse_proxy_ref_tokens("#1-#3").unwrap(), vec!["1", "2", "3"]);
    }

    #[test]
    fn failover_disables_secondary_stubs() {
        let p1 = ProxyTunnelConfig {
            id: "a".into(),
            name: "one".into(),
            enabled: true,
            proxy_type: ProxyType::Socks5,
            host: "h".into(),
            port: 1080,
            username: String::new(),
            password: String::new(),
            test_target: None,
            profile_id: String::new(),
        };
        let p2 = ProxyTunnelConfig {
            id: "b".into(),
            name: "two".into(),
            enabled: true,
            proxy_type: ProxyType::Socks5,
            host: "h2".into(),
            port: 1080,
            username: String::new(),
            password: String::new(),
            test_target: None,
            profile_id: String::new(),
        };
        let config: crate::models::connection::ConnectionConfig = serde_json::from_value(serde_json::json!({
            "id": "c",
            "name": "c",
            "db_type": "mysql",
            "host": "db",
            "port": 3306,
            "username": "",
            "password": ""
        }))
        .expect("minimal connection");
        let out = apply_proxy_profiles_failover(config, &[&p1, &p2]);
        let proxies: Vec<_> = out
            .transport_layers
            .iter()
            .filter_map(|layer| match layer {
                TransportLayerConfig::Proxy(proxy) => Some(proxy),
                _ => None,
            })
            .collect();
        assert_eq!(proxies.len(), 2);
        assert!(proxies[0].enabled);
        assert!(!proxies[1].enabled);
        assert_eq!(proxies[0].profile_id, "a");
        assert_eq!(proxies[1].profile_id, "b");
    }
}
