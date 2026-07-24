import { getRemoteApiBaseUrl } from "@/lib/backend/remoteApiConfig";

function normalizeBasePath(value: string | undefined): string {
  const trimmed = (value ?? "").trim();
  if (!trimmed || trimmed === "." || trimmed === "./" || trimmed === "/") return "";
  const withoutQuery = trimmed.split(/[?#]/, 1)[0] ?? "";
  const withLeadingSlash = withoutQuery.startsWith("/") ? withoutQuery : `/${withoutQuery}`;
  return withLeadingSlash.replace(/\/+$/, "");
}

function inferredRuntimeBasePath(pathname: string): string {
  const normalized = pathname.replace(/\/+$/, "");
  if (!normalized || normalized === "/login") return "";
  if (normalized.endsWith("/login")) return normalized.slice(0, -"/login".length);
  return normalized;
}

export function dbxWebBasePath(pathname = globalThis.location?.pathname ?? "", buildBase = import.meta.env.BASE_URL): string {
  const configured = normalizeBasePath(buildBase);
  if (configured) return configured;
  return normalizeBasePath(inferredRuntimeBasePath(pathname));
}

export function webPath(path: string, basePath = dbxWebBasePath()): string {
  const normalizedPath = path.startsWith("/") ? path : `/${path}`;
  const base = normalizeBasePath(basePath);
  return `${base}${normalizedPath}` || "/";
}

function normalizeApiPath(path: string): string {
  const pathWithLeadingSlash = path.startsWith("/") ? path : `/${path}`;
  return pathWithLeadingSlash === "/api" || pathWithLeadingSlash.startsWith("/api/") || pathWithLeadingSlash.startsWith("/api?") ? pathWithLeadingSlash : `/api${pathWithLeadingSlash}`;
}

export function apiUrl(path: string, basePath = dbxWebBasePath()): string {
  const normalizedPath = normalizeApiPath(path);
  const remoteBase = getRemoteApiBaseUrl();
  if (remoteBase) return `${remoteBase}${normalizedPath}`;
  return webPath(normalizedPath, basePath);
}

type WebSocketLocation = Pick<Location, "protocol" | "host"> | undefined;

export function apiWebSocketUrl(path: string, basePath = dbxWebBasePath(), currentLocation: WebSocketLocation = globalThis.location): string {
  const normalizedPath = normalizeApiPath(path);
  const remoteBase = getRemoteApiBaseUrl();
  if (remoteBase) {
    const remote = new URL(remoteBase.includes("://") ? remoteBase : `http://${remoteBase}`);
    const protocol = remote.protocol === "https:" ? "wss:" : "ws:";
    const basePathPart = remote.pathname.replace(/\/+$/, "");
    return `${protocol}//${remote.host}${basePathPart}${normalizedPath}`;
  }
  const protocol = currentLocation?.protocol === "https:" ? "wss:" : "ws:";
  return `${protocol}//${currentLocation?.host ?? ""}${webPath(normalizedPath, basePath)}`;
}
