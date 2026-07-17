import { randomUUID } from "node:crypto";
import { existsSync } from "node:fs";
import Database from "better-sqlite3";
import type { ProxyTunnelConfig, TransportLayerConfig } from "./connections.js";
import { parseListIndex, resolveProxyProfileByIndex } from "./list-index.js";
import { dbPath as defaultDbPath } from "./paths.js";

export type TunnelProfile = TransportLayerConfig;

export interface TunnelProfileStoreOptions {
  path?: string;
}

function openDb(readonly = false, path = defaultDbPath()): Database.Database {
  return new Database(path, { readonly });
}

function tunnelProfilesTableExists(db: Database.Database): boolean {
  const row = db.prepare("SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?").get("tunnel_profiles") as { "1": number } | undefined;
  return !!row;
}

export async function loadTunnelProfiles(options: TunnelProfileStoreOptions = {}): Promise<TunnelProfile[]> {
  const path = options.path ?? defaultDbPath();
  if (!existsSync(path)) return [];

  const db = openDb(true, path);
  try {
    if (!tunnelProfilesTableExists(db)) return [];
    const rows = db.prepare("SELECT config_json FROM tunnel_profiles ORDER BY rowid").all() as { config_json: string }[];
    return rows.map((row) => JSON.parse(row.config_json) as TunnelProfile);
  } finally {
    db.close();
  }
}

export async function findTunnelProfile(id: string, options: TunnelProfileStoreOptions = {}): Promise<TunnelProfile | undefined> {
  const trimmed = id.trim();
  if (!trimmed) return undefined;
  const profiles = await loadTunnelProfiles(options);
  return profiles.find((profile) => profile.id === trimmed);
}

export function isProxyTunnelProfile(profile: TunnelProfile): profile is { type: "proxy" } & ProxyTunnelConfig {
  return profile.type === "proxy";
}

export interface ProxyProfileRefArgs {
  proxy_profile_id?: string;
  proxy_profile_name?: string;
}

export interface InlineProxyArgs {
  proxy_enabled?: boolean;
  proxy_host?: string;
  proxy_port?: number;
  proxy_username?: string;
  proxy_password?: string;
}

export function hasProxyProfileRef(args: ProxyProfileRefArgs): boolean {
  return !!(args.proxy_profile_id?.trim() || args.proxy_profile_name?.trim());
}

export function hasInlineProxyParams(args: InlineProxyArgs): boolean {
  return (
    !!args.proxy_enabled ||
    !!args.proxy_host?.trim() ||
    args.proxy_port !== undefined ||
    !!args.proxy_username?.trim() ||
    !!args.proxy_password
  );
}

export function findProxyProfilesByName(profiles: TunnelProfile[], name: string): TunnelProfile[] {
  const lower = name.trim().toLowerCase();
  return profiles.filter((profile) => profile.type === "proxy" && (profile.name || "").toLowerCase() === lower);
}

export function findProxyProfile(profiles: TunnelProfile[], args: ProxyProfileRefArgs): TunnelProfile | undefined {
  if (args.proxy_profile_id?.trim()) {
    const trimmed = args.proxy_profile_id.trim();
    const match = profiles.find((profile) => profile.id === trimmed);
    if (match?.type === "proxy") return match;
    const listIndex = parseListIndex(trimmed);
    if (listIndex !== undefined) return resolveProxyProfileByIndex(profiles, listIndex);
    return undefined;
  }
  if (args.proxy_profile_name?.trim()) {
    const trimmed = args.proxy_profile_name.trim();
    const matches = findProxyProfilesByName(profiles, trimmed);
    if (matches.length === 1) return matches[0];
    const listIndex = parseListIndex(trimmed);
    if (listIndex !== undefined) return resolveProxyProfileByIndex(profiles, listIndex);
    return undefined;
  }
  return undefined;
}

export function proxyProfileSummary(profile: { type: "proxy" } & ProxyTunnelConfig): string {
  if (profile.type !== "proxy") return profile.name || profile.id;
  if (!profile.host) return profile.name || profile.id;
  return `${profile.proxy_type || "socks5"}://${profile.host}:${profile.port || 1080}`;
}

/**
 * Builds the reference stub stored on a connection for a shared proxy profile.
 * Credentials stay in the profile; the backend resolves the reference at connect time.
 */
export function proxyProfileReferenceLayer(profile: { type: "proxy" } & ProxyTunnelConfig, layerId: string): TransportLayerConfig {
  return {
    type: "proxy",
    id: layerId,
    name: profile.name || "",
    enabled: profile.enabled !== false,
    profile_id: profile.id,
    proxy_type: profile.proxy_type || "socks5",
    host: "",
    port: 1080,
    username: "",
    password: "",
  };
}

type LegacyProxyFields = {
  proxy_enabled?: boolean;
  proxy_type?: "socks5" | "http";
  proxy_host?: string;
  proxy_port?: number;
  proxy_username?: string;
  proxy_password?: string;
};

/**
 * Replace any existing proxy (inline or profile stub) with a saved-profile reference.
 * Non-proxy layers (e.g. SSH) are preserved. Legacy proxy_* fields are cleared so
 * they cannot reintroduce a stacked proxy via normalizeTransportLayers.
 * Does not persist — callers decide whether to save or use for one request.
 */
export function applyProxyProfileOverride<T extends { transport_layers?: TransportLayerConfig[] }>(
  config: T,
  profile: { type: "proxy" } & ProxyTunnelConfig,
  layerId: string = randomUUID(),
): T {
  const existing = Array.isArray(config.transport_layers) ? config.transport_layers : [];
  const kept = existing.filter((layer) => layer.type !== "proxy");
  const next = {
    ...config,
    transport_layers: [...kept, proxyProfileReferenceLayer(profile, layerId)],
  } as T & LegacyProxyFields;
  next.proxy_enabled = false;
  delete next.proxy_type;
  delete next.proxy_host;
  delete next.proxy_port;
  delete next.proxy_username;
  delete next.proxy_password;
  return next;
}
