import { safeLocalStorageGet, safeLocalStorageSet } from "@/lib/backend/safeStorage";

/** Aligned with CLI/MCP env names for localStorage and Vite overrides. */
export const REMOTE_API_URL_KEY = "DBX_WEB_URL";
export const REMOTE_API_PASSWORD_KEY = "DBX_WEB_PASSWORD";

function readEnvOverride(name: "DBX_WEB_URL" | "DBX_WEB_PASSWORD"): string {
  const meta = import.meta.env as Record<string, string | undefined>;
  const viteKey = name === "DBX_WEB_URL" ? "VITE_DBX_WEB_URL" : "VITE_DBX_WEB_PASSWORD";
  const fromVite = (meta[viteKey] || meta[name] || "").trim();
  if (fromVite) return fromVite;
  const injected = (globalThis as Record<string, unknown>)[`__${name}__`];
  return typeof injected === "string" ? injected.trim() : "";
}

export function normalizeRemoteApiBaseUrl(raw: string): string {
  return raw.trim().replace(/\/+$/, "");
}

/** Effective remote API base URL (env override > localStorage). Empty = local/Tauri backend. */
export function getRemoteApiBaseUrl(): string {
  const fromEnv = readEnvOverride("DBX_WEB_URL");
  if (fromEnv) return normalizeRemoteApiBaseUrl(fromEnv);
  return normalizeRemoteApiBaseUrl(safeLocalStorageGet(REMOTE_API_URL_KEY) || "");
}

export function getRemoteApiPassword(): string {
  const fromEnv = readEnvOverride("DBX_WEB_PASSWORD");
  if (fromEnv) return fromEnv;
  return safeLocalStorageGet(REMOTE_API_PASSWORD_KEY) || "";
}

export function isRemoteApiConfigured(): boolean {
  return getRemoteApiBaseUrl().length > 0;
}

export function loadRemoteApiSettings(): { url: string; password: string } {
  return {
    url: safeLocalStorageGet(REMOTE_API_URL_KEY) || "",
    password: safeLocalStorageGet(REMOTE_API_PASSWORD_KEY) || "",
  };
}

export function saveRemoteApiSettings(url: string, password: string): void {
  const normalized = normalizeRemoteApiBaseUrl(url);
  if (normalized) {
    safeLocalStorageSet(REMOTE_API_URL_KEY, normalized);
  } else {
    safeLocalStorageSet(REMOTE_API_URL_KEY, "");
  }
  safeLocalStorageSet(REMOTE_API_PASSWORD_KEY, password);
}

export function clearRemoteApiSettings(): void {
  safeLocalStorageSet(REMOTE_API_URL_KEY, "");
  safeLocalStorageSet(REMOTE_API_PASSWORD_KEY, "");
}
