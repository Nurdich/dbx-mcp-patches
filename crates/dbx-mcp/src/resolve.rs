//! Connection / proxy resolution helpers used by MCP tools.

use dbx_core::models::connection::{ConnectionConfig, ProxyTunnelConfig, TransportLayerConfig};
use dbx_core::storage::McpGlobalPolicy;
use rmcp::model::{CallToolResult, ContentBlock};

use crate::backend::DbxBackend;
use crate::list_index::{parse_list_index, parse_list_index_range};
use crate::server::McpScope;
use crate::tunnel_profiles::{
    apply_proxy_profiles_failover, has_proxy_profile_ref, resolve_proxy_profiles, with_env_defaults,
    ProxyProfileRefArgs,
};

fn tool_error(code: &str, message: impl Into<String>) -> CallToolResult {
    CallToolResult::error(vec![ContentBlock::text(format!("Error [{code}]: {}", message.into()))])
}

pub fn policy_allows_connection(policy: &McpGlobalPolicy, connection: &ConnectionConfig) -> bool {
    policy
        .allowed_connection_ids
        .as_ref()
        .is_none_or(|allowed| allowed.iter().any(|id| id == &connection.id))
}

fn filter_scoped(
    connections: Vec<ConnectionConfig>,
    scope: &McpScope,
    policy: &McpGlobalPolicy,
) -> Vec<ConnectionConfig> {
    connections
        .into_iter()
        .filter(|connection| policy_allows_connection(policy, connection))
        .filter(|connection| !scope.connection_scope_enabled() || scope.matches(connection))
        .collect()
}

/// Resolve one or many connections from id/name/index/range tokens.
pub async fn resolve_connections(
    backend: &dyn DbxBackend,
    scope: &McpScope,
    connection_id: Option<&str>,
    connection_name: Option<&str>,
) -> Result<Vec<ConnectionConfig>, CallToolResult> {
    let policy = backend
        .load_mcp_global_policy()
        .await
        .map_err(|error| tool_error("MCP_POLICY_UNAVAILABLE", error))?;
    let connections = backend
        .load_connections()
        .await
        .map_err(|error| tool_error("CONNECTION_LOAD_ERROR", error))?;
    let scoped = filter_scoped(connections, scope, &policy);

    if let Some(id) = connection_id.map(str::trim).filter(|id| !id.is_empty()) {
        if let Some(indexes) =
            parse_list_index_range(id).map_err(|error| tool_error("INVALID_LIST_INDEX_RANGE", error.message))?
        {
            return resolve_by_indexes(&scoped, &indexes);
        }
        let connection = scoped
            .into_iter()
            .find(|connection| connection.id == id)
            .ok_or_else(|| tool_error("CONNECTION_NOT_FOUND", format!("Connection with id \"{id}\" not found.")))?;
        return Ok(vec![connection]);
    }

    if scope.connection_scope_enabled() {
        let connection = scoped
            .into_iter()
            .find(|connection| scope.matches(connection))
            .ok_or_else(|| tool_error("CONNECTION_NOT_FOUND", "Scoped DBX connection was not found."))?;
        if let Some(name) = connection_name.map(str::trim).filter(|name| !name.is_empty()) {
            if name != connection.name && name != connection.id && parse_list_index(name) != Some(1) {
                return Err(tool_error(
                    "CONNECTION_OUT_OF_SCOPE",
                    format!("Connection \"{name}\" is outside this DBX AI session scope."),
                ));
            }
        }
        return Ok(vec![connection]);
    }

    let Some(name) = connection_name.map(str::trim).filter(|name| !name.is_empty()) else {
        return Err(tool_error("CONNECTION_NOT_FOUND", "Either connection_id or connection_name is required."));
    };

    if let Some(indexes) =
        parse_list_index_range(name).map_err(|error| tool_error("INVALID_LIST_INDEX_RANGE", error.message))?
    {
        return resolve_by_indexes(&scoped, &indexes);
    }

    let matching: Vec<_> =
        scoped.into_iter().filter(|connection| connection.name.eq_ignore_ascii_case(name)).collect();
    match matching.as_slice() {
        [] => Err(tool_error("CONNECTION_NOT_FOUND", format!("Connection \"{name}\" not found."))),
        [connection] => Ok(vec![connection.clone()]),
        many => {
            let lines = many
                .iter()
                .map(|connection| {
                    format!("- {}: {:?} @ {}:{}", connection.id, connection.db_type, connection.host, connection.port)
                })
                .collect::<Vec<_>>()
                .join("\n");
            Err(tool_error(
                "AMBIGUOUS_CONNECTION",
                format!("Multiple connections found with name \"{name}\". Please specify connection_id:\n{lines}"),
            ))
        }
    }
}

fn resolve_by_indexes(scoped: &[ConnectionConfig], indexes: &[usize]) -> Result<Vec<ConnectionConfig>, CallToolResult> {
    let mut out = Vec::with_capacity(indexes.len());
    for index in indexes {
        let Some(connection) = scoped.get(index - 1) else {
            return Err(tool_error(
                "CONNECTION_NOT_FOUND",
                format!("List index #{index} is out of range (1-{}).", scoped.len()),
            ));
        };
        out.push(connection.clone());
    }
    Ok(out)
}

pub async fn resolve_single_connection(
    backend: &dyn DbxBackend,
    scope: &McpScope,
    connection_id: Option<&str>,
    connection_name: Option<&str>,
) -> Result<ConnectionConfig, CallToolResult> {
    let configs = resolve_connections(backend, scope, connection_id, connection_name).await?;
    match configs.as_slice() {
        [] => Err(tool_error("CONNECTION_NOT_FOUND", "No connection matched.")),
        [connection] => Ok(connection.clone()),
        _ => Err(tool_error(
            "CONNECTION_RANGE",
            format!(
                "Connection range resolves to {} connections. This tool accepts a single connection; use a single index/name, or a batch-capable tool (dbx_get_database_stats / dbx_get_database_report / dbx_execute_query).",
                configs.len()
            ),
        )),
    }
}

pub async fn apply_proxy_override_if_requested(
    backend: &dyn DbxBackend,
    config: ConnectionConfig,
    proxy_profile_id: Option<String>,
    proxy_profile_name: Option<String>,
) -> Result<ConnectionConfig, CallToolResult> {
    apply_proxy_override_with_args(
        backend,
        config,
        ProxyProfileRefArgs {
            proxy_profile_id,
            proxy_profile_name,
            proxy_profile_ids: None,
            proxy_profile_names: None,
        },
    )
    .await
}

pub async fn apply_proxy_override_with_args(
    backend: &dyn DbxBackend,
    config: ConnectionConfig,
    args: ProxyProfileRefArgs,
) -> Result<ConnectionConfig, CallToolResult> {
    let args = with_env_defaults(args);
    if !has_proxy_profile_ref(&args) {
        return Ok(config);
    }

    let has_id = args.proxy_profile_id.as_deref().is_some_and(|v| !v.trim().is_empty())
        || args.proxy_profile_ids.as_ref().is_some_and(|v| v.iter().any(|s| !s.trim().is_empty()));
    let has_name = args.proxy_profile_name.as_deref().is_some_and(|v| !v.trim().is_empty())
        || args.proxy_profile_names.as_ref().is_some_and(|v| v.iter().any(|s| !s.trim().is_empty()));
    if has_id && has_name {
        return Err(tool_error(
            "PROXY_CONFLICT",
            "Specify either proxy_profile_id(s) or proxy_profile_name(s), not both.",
        ));
    }

    let profiles = backend
        .load_tunnel_profiles()
        .await
        .map_err(|error| tool_error("PROXY_LOAD_ERROR", error))?;

    let resolved = match resolve_proxy_profiles(&profiles, &args) {
        Ok(list) if !list.is_empty() => list,
        Ok(_) => {
            return Err(tool_error(
                "PROXY_PROFILE_NOT_FOUND",
                "Proxy profile not found. Use dbx_list_proxies to see saved profiles from DBX Settings > Tunnels.",
            ))
        }
        Err(message) => {
            let code = if message.contains("Multiple proxy profiles") {
                "AMBIGUOUS_PROXY_PROFILE"
            } else if message.contains("out of range") || message.contains("not found") {
                "PROXY_PROFILE_NOT_FOUND"
            } else {
                "INVALID_PROXY_PROFILE"
            };
            return Err(tool_error(code, message));
        }
    };

    let mut proxy_configs = Vec::with_capacity(resolved.len());
    for profile in &resolved {
        let TransportLayerConfig::Proxy(proxy) = profile else {
            return Err(tool_error("PROXY_PROFILE_NOT_FOUND", "Selected tunnel profile is not a proxy profile."));
        };
        proxy_configs.push(proxy);
    }

    Ok(apply_proxy_profiles_failover(config, &proxy_configs))
}

pub fn as_proxy_config(profile: &TransportLayerConfig) -> Option<&ProxyTunnelConfig> {
    match profile {
        TransportLayerConfig::Proxy(proxy) => Some(proxy),
        _ => None,
    }
}
