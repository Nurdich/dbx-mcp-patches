import { apiUrl } from "@/lib/common/webPath";
import { getRemoteApiBaseUrl, getRemoteApiPassword, isRemoteApiConfigured } from "@/lib/backend/remoteApiConfig";

interface AuthCheckResponse {
  authenticated: boolean;
  required: boolean;
  setup_required: boolean;
}

interface LoginResponse {
  ok?: boolean;
  session?: string;
  error?: string;
}

let sessionToken: string | null = null;
let authNotRequired = false;
let authInFlight: Promise<void> | null = null;

export function getRemoteSessionToken(): string | null {
  return sessionToken;
}

export function clearRemoteSession(): void {
  sessionToken = null;
  authNotRequired = false;
  authInFlight = null;
}

function appendSessionQuery(url: string, token: string | null): string {
  if (!token) return url;
  const joiner = url.includes("?") ? "&" : "?";
  return `${url}${joiner}dbx_session=${encodeURIComponent(token)}`;
}

/** EventSource cannot set custom headers; pass session as query when remote. */
export function withRemoteSessionQuery(url: string): string {
  if (!isRemoteApiConfigured()) return url;
  return appendSessionQuery(url, sessionToken);
}

export function createApiEventSource(path: string): EventSource {
  return new EventSource(withRemoteSessionQuery(apiUrl(path)));
}

async function loginRemote(password: string): Promise<string> {
  const res = await fetch(apiUrl("/api/auth/login"), {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ password }),
  });
  if (res.status === 401) {
    throw new Error("Remote API login failed: incorrect password (or rate-limited).");
  }
  if (!res.ok) {
    const details = await res.text().catch(() => "");
    throw new Error(`Remote API login failed: ${res.status} ${details}`.trim());
  }
  const body = (await res.json().catch(() => ({}))) as LoginResponse;
  if (typeof body.session === "string" && body.session) {
    return body.session;
  }
  throw new Error("Remote API login succeeded but no session token was returned.");
}

async function ensureRemoteAuthUnlocked(): Promise<void> {
  if (!isRemoteApiConfigured()) return;
  if (sessionToken || authNotRequired) return;

  const checkRes = await fetch(apiUrl("/api/auth/check"));
  if (!checkRes.ok) {
    throw new Error(`Remote API auth check failed: ${checkRes.status}`);
  }
  const check = (await checkRes.json()) as AuthCheckResponse;
  if (check.setup_required) {
    throw new Error("Remote DBX Web password setup is required before API access.");
  }
  if (!check.required) {
    authNotRequired = true;
    sessionToken = null;
    return;
  }
  const password = getRemoteApiPassword();
  if (!password) {
    throw new Error(`Remote API authentication is required for ${getRemoteApiBaseUrl()}. Set the password in Settings → Remote API (same as DBX_WEB_PASSWORD).`);
  }
  sessionToken = await loginRemote(password);
  authNotRequired = false;
}

export async function ensureRemoteAuth(): Promise<void> {
  if (!isRemoteApiConfigured()) return;
  if (sessionToken || authNotRequired) return;
  if (!authInFlight) {
    authInFlight = ensureRemoteAuthUnlocked().finally(() => {
      authInFlight = null;
    });
  }
  await authInFlight;
}

function applySessionHeader(headers: Headers, token: string | null): void {
  if (token) headers.set("X-DBX-Session", token);
}

/**
 * Fetch against apiUrl(path), with remote session header + 401 re-login when Remote API is configured.
 * Same-origin web mode is unchanged (browser cookies still apply).
 */
export async function apiFetch(path: string, init?: RequestInit): Promise<Response> {
  const run = async (): Promise<Response> => {
    await ensureRemoteAuth();
    const headers = new Headers(init?.headers);
    if (isRemoteApiConfigured()) {
      applySessionHeader(headers, sessionToken);
    }
    return fetch(apiUrl(path), { ...init, headers });
  };

  let res = await run();
  if (res.status === 401 && isRemoteApiConfigured() && getRemoteApiPassword()) {
    clearRemoteSession();
    res = await run();
  }
  return res;
}
