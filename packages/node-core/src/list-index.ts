import type { ConnectionConfig } from "./connections.js";
import type { TunnelProfile } from "./tunnel-profiles.js";

/** Parse a 1-based list index from `1`, `#2`, etc. Returns undefined if not a list index token. */
export function parseListIndex(value: string): number | undefined {
  const trimmed = value.trim();
  if (!trimmed) return undefined;
  const match = trimmed.match(/^#?(\d+)$/);
  if (!match) return undefined;
  const index = Number(match[1]);
  return Number.isInteger(index) && index >= 1 ? index : undefined;
}

export function itemAtListIndex<T>(items: readonly T[], index: number): T | undefined {
  if (!Number.isInteger(index) || index < 1 || index > items.length) return undefined;
  return items[index - 1];
}

export function resolveConnectionByIndex(connections: readonly ConnectionConfig[], index: number): ConnectionConfig | undefined {
  return itemAtListIndex(connections, index);
}

export function listProxyProfiles(profiles: readonly TunnelProfile[]): TunnelProfile[] {
  return profiles.filter((profile) => profile.type === "proxy");
}

export function resolveProxyProfileByIndex(profiles: readonly TunnelProfile[], index: number): TunnelProfile | undefined {
  return itemAtListIndex(listProxyProfiles(profiles), index);
}

export function formatListIndex(index: number): string {
  return String(index);
}
