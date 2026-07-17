import { randomUUID } from "node:crypto";
import { existsSync } from "node:fs";
import Database from "better-sqlite3";
import { dbPath } from "@dbx-app/node-core";

export interface TunnelProfile {
  type: "proxy" | "ssh" | "http_tunnel";
  id: string;
  name?: string;
  enabled?: boolean;
  proxy_type?: "socks5" | "http";
  host?: string;
  port?: number;
  username?: string;
  password?: string;
  url?: string;
  token?: string;
}

export interface ProxyProfileRefArgs {
  proxy_profile_id?: string;
  proxy_profile_name?: string;
}

export interface InlineProxyArgs {
  proxy_enabled: boolean;
  proxy_host?: string;
  proxy_port?: number;
  proxy_username?: string;
  proxy_password?: string;
}

let webSessionCookie: string | null = null;
let webAuthChecked = false;

function webBaseUrl(): string {
  return process.env.DBX_WEB_URL!.replace(/\/+$/, "");
}

function webPassword(): string {
  return process.env.DBX_WEB_PASSWORD || "";
}

function extractSessionCookie(setCookie: string | null): string | null {
  const match = setCookie?.match(/dbx_session=([^;]+)/);
  return match?.[1] ?? null;
}

async function ensureWebAuth(): Promise<void> {
  if (webSessionCookie || webAuthChecked) return;

  const res = await fetch(`${webBaseUrl()}/api/auth/check`, { method: "GET", redirect: "manual" });
  if (!res.ok) {
    throw new Error(`Authentication check failed: ${res.status} ${res.statusText}`);
  }
  const auth = (await res.json()) as { setup_required?: boolean; required?: boolean; authenticated?: boolean };
  if (auth.setup_required) {
    throw new Error("DBX Web password setup is required before MCP Web mode can access APIs.");
  }
  if (!auth.required || auth.authenticated) {
    webAuthChecked = true;
    return;
  }
  const password = webPassword();
  if (!password) {
    throw new Error("DBX Web authentication is required. Set DBX_WEB_PASSWORD for MCP Web mode.");
  }
  const login = await fetch(`${webBaseUrl()}/api/auth/login`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ password }),
    redirect: "manual",
  });
  if (!login.ok) {
    throw new Error(`Authentication failed: ${login.status} ${login.statusText}`);
  }
  webSessionCookie = extractSessionCookie(login.headers.get("set-cookie"));
  if (!webSessionCookie) {
    throw new Error("Authentication failed: DBX Web did not return a session cookie.");
  }
  webAuthChecked = true;
}

async function webApiFetch(path: string): Promise<Response> {
  await ensureWebAuth();
  const headers: Record<string, string> = { "Content-Type": "application/json" };
  if (webSessionCookie) headers.Cookie = `dbx_session=${webSessionCookie}`;

  let res = await fetch(`${webBaseUrl()}${path}`, { headers });
  if (res.status === 401 && webSessionCookie && webPassword()) {
    webSessionCookie = null;
    webAuthChecked = false;
    await ensureWebAuth();
    const retryHeaders = { ...headers };
    if (webSessionCookie) retryHeaders.Cookie = `dbx_session=${webSessionCookie}`;
    res = await fetch(`${webBaseUrl()}${path}`, { headers: retryHeaders });
  }
  if (!res.ok) {
    const body = await res.text().catch(() => "");
    throw new Error(`API request ${path} failed: ${res.status} ${res.statusText} ${body}`);
  }
  return res;
}

export function loadTunnelProfilesFromDb(path = dbPath()): TunnelProfile[] {
  if (!existsSync(path)) return [];

  const db = new Database(path, { readonly: true });
  try {
    const tableExists = db
      .prepare("SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'tunnel_profiles'")
      .get();
    if (!tableExists) return [];

    const rows = db.prepare("SELECT config_json FROM tunnel_profiles ORDER BY rowid").all() as { config_json: string }[];
    const profiles: TunnelProfile[] = [];
    for (const row of rows) {
      try {
        profiles.push(JSON.parse(row.config_json) as TunnelProfile);
      } catch {
        // Skip malformed rows, matching DBX storage behavior.
      }
    }
    return profiles;
  } finally {
    db.close();
  }
}

export async function loadTunnelProfilesWeb(): Promise<TunnelProfile[]> {
  const res = await webApiFetch("/api/tunnel-profiles/list");
  return (await res.json()) as TunnelProfile[];
}

export async function loadTunnelProfiles(isWebMode: boolean): Promise<TunnelProfile[]> {
  if (isWebMode) return loadTunnelProfilesWeb();
  return loadTunnelProfilesFromDb();
}

export function hasProxyProfileRef(args: ProxyProfileRefArgs): boolean {
  return !!(args.proxy_profile_id?.trim() || args.proxy_profile_name?.trim());
}

export function hasInlineProxyParams(args: InlineProxyArgs): boolean {
  return (
    args.proxy_enabled ||
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

function parseListIndex(value: string): number | undefined {
  const trimmed = value.trim();
  if (!trimmed) return undefined;
  const match = trimmed.match(/^#?(\d+)$/);
  if (!match) return undefined;
  const index = Number(match[1]);
  return Number.isInteger(index) && index >= 1 ? index : undefined;
}

function resolveProxyProfileByIndex(profiles: TunnelProfile[], index: number): TunnelProfile | undefined {
  const proxies = profiles.filter((profile) => profile.type === "proxy");
  if (!Number.isInteger(index) || index < 1 || index > proxies.length) return undefined;
  return proxies[index - 1];
}

export function proxyProfileSummary(profile: TunnelProfile): string {
  if (profile.type !== "proxy") return profile.name || profile.id;
  if (!profile.host) return profile.name || profile.id;
  return `${profile.proxy_type || "socks5"}://${profile.host}:${profile.port || 1080}`;
}

export function buildProxyProfileReferenceLayer(profile: TunnelProfile): Record<string, unknown> {
  return {
    type: "proxy",
    id: randomUUID(),
    enabled: true,
    name: profile.name || "",
    profile_id: profile.id,
    proxy_type: profile.proxy_type || "socks5",
    host: "",
    port: profile.port || 1080,
    username: "",
    password: "",
  };
}
