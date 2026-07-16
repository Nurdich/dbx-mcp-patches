import type { ConnectionConfig } from "./connections.js";
import type { TunnelProfile } from "./tunnel-profiles.js";

/** Maximum end index allowed in a numeric range reference (e.g. `1-15`). */
export const MAX_LIST_INDEX_RANGE_END = 15;

/** Maximum number of connections in a single range batch. */
export const MAX_LIST_INDEX_RANGE_SIZE = 15;

export class ListIndexRangeError extends Error {
  readonly code = "INVALID_LIST_INDEX_RANGE";

  constructor(message: string) {
    super(message);
    this.name = "ListIndexRangeError";
  }
}

/** Parse a 1-based list index from `1`, `#2`, etc. Returns undefined if not a list index token. */
export function parseListIndex(value: string): number | undefined {
  const trimmed = value.trim();
  if (!trimmed) return undefined;
  const match = trimmed.match(/^#?(\d+)$/);
  if (!match) return undefined;
  const index = Number(match[1]);
  return Number.isInteger(index) && index >= 1 ? index : undefined;
}

/**
 * Parse a single index or inclusive range: `1`, `#2`, `1-15`, `1..15`, `1:15`, `#1-#15`.
 * Returns undefined when the token is not numeric/range syntax (e.g. a connection name or UUID).
 */
export function parseListIndexRange(value: string): number[] | undefined {
  const trimmed = value.trim();
  if (!trimmed) return undefined;

  const single = parseListIndex(trimmed);
  if (single !== undefined) return [single];

  const match = trimmed.match(/^#?(\d+)\s*(?:-|\.{2}|:)\s*#?(\d+)$/);
  if (!match) return undefined;

  const start = Number(match[1]);
  const end = Number(match[2]);
  if (!Number.isInteger(start) || !Number.isInteger(end)) return undefined;
  if (start < 1) {
    throw new ListIndexRangeError(`Range start must be >= 1. Got ${start}.`);
  }
  if (end > MAX_LIST_INDEX_RANGE_END) {
    throw new ListIndexRangeError(`Range end must be <= ${MAX_LIST_INDEX_RANGE_END}. Got ${end}.`);
  }
  if (start > end) {
    throw new ListIndexRangeError(`Invalid range: start (${start}) must be <= end (${end}).`);
  }

  const size = end - start + 1;
  if (size > MAX_LIST_INDEX_RANGE_SIZE) {
    throw new ListIndexRangeError(
      `Range cannot include more than ${MAX_LIST_INDEX_RANGE_SIZE} connections. Got ${size} (${start}-${end}).`,
    );
  }

  return Array.from({ length: size }, (_, i) => start + i);
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
