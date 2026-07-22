# DBX MCP / CLI Patches (Rust era — 2026-07-22)

Local enhancements on top of upstream [t8y2/dbx](https://github.com/t8y2/dbx) **Rust** MCP/CLI + **dbx-core** failover / MCP-update runtime subset.

## Upstream baseline

| Item | Value |
|------|-------|
| Upstream repo | https://github.com/t8y2/dbx |
| Base commit | `1b399a58` — fix(updater): sync release notes (main @ 2026-07-22) |
| Nearby tags | `v0.5.63`, packages `0.4.40` |
| MCP | `crates/dbx-mcp` 0.4.40 |
| CLI | `crates/dbx-cli` 0.4.40 |
| Core | `crates/dbx-core` subset (failover/progress/storage update) — [APPLY.md](./crates/dbx-core/APPLY.md) |
| npm | Thin launchers only (`packages/mcp-server`, `packages/cli`) |
| Node 0.4.x | **Abandoned** — see [LEGACY.md](./LEGACY.md); tree `legacy-node-packages/` removed |
| Not in this repo | `dbx-web` (`POST /connection/mcp/update`) — deploy from full monorepo |

Previous Node baseline (`packages-v0.4.31` / `5206750`) is no longer maintained; all features live in Rust crates **including** the `dbx-core` modules shipped here.

## What we patched (not in upstream Rust)

### Core (`crates/dbx-core` subset)

| Feature | Status |
|---------|--------|
| Multi-proxy **failover group** runtime (`connection_host_port`) | **In patches** |
| `connect_progress` hook + emit | **In patches** |
| `verify_proxy_connect` for pure-proxy failover | **In patches** |
| `Storage::update_connection_for_mcp` (winner writeback) | **In patches** |

See [crates/dbx-core/APPLY.md](./crates/dbx-core/APPLY.md) and optional [`patches/dbx-core-failover.patch`](./patches/dbx-core-failover.patch).

### MCP (`crates/dbx-mcp`)

| Feature | Status |
|---------|--------|
| `dbx_list_proxies` | **Ported** |
| `dbx_get_database_stats` / `dbx_get_database_report` | **Ported** |
| Numeric `#` / ranges on connection selectors | **Ported** |
| `#` column in `dbx_list_connections` | **Ported** |
| Inline proxy + `proxy_profile_*` on `dbx_add_connection` | **Ported** |
| Multi-proxy **failover group** (try-next, not chain) | **Ported** (MCP + CLI + `dbx-core`) |
| One-shot `proxy_profile_*` on stats/report/**query** | **Ported** |
| Process-wide `DBX_PROXY_PROFILE_IDS` / `NAMES` defaults | **Ported** |
| `dbx_execute_query` `timeout_ms` → query timeout | **Ported** (via `backend.execute_query`) |
| Batch ranges on list_tables / describe / query / schema_context / stats / report | **Ported** (sequential) |
| `skip_unsupported` + Skipped vs Failures | **Ported** |
| Progress prepend in tool text (`DBX_MCP_QUIET` / `DBX_MCP_VERBOSE`) | **Ported** |
| Web auth harden (`dbx_session` required) + duckdb note | **Ported** (auth in mcp; duckdb in monorepo Cargo.toml) |
| `update_connection_for_mcp` backend + Web API path | **Ported** (backend/CLI); **web route only in monorepo** |
| Parallel batch (MCP) | N/A (CLI-only) |

### CLI (`crates/dbx-cli`)

| Feature | Status |
|---------|--------|
| `dbx proxies list` / `stats` / `report` | **Ported** |
| List index / ranges + `#` column | **Ported** |
| Report save to `{cwd}/reports/` (`-n` / `-o`) | **Ported** |
| `--parallel` / `-P` (default concurrency 15) | **Ported** |
| Short flags (`-j -d -s -t -P -n -o -v -q -H` …) | **Ported** |
| `connections add` / `connections remove` | **Ported** |
| `connections update` range + multi-proxy failover writeback | **Ported** |
| `connections import` bulk JSON write (no probe) | **Ported** |
| `dbx redis` | **Ported** |
| Proxy override on stats/report/query | **Ported** |
| Streaming stderr progress + batch soft-fail exit | **Ported** |
| Continue-on-error + Skipped vs Failures | **Ported** |

### Shared modules (inside `dbx-mcp` crate)

- `list_index.rs` — `1` / `#2` / `1-15` / `1..15`
- `tunnel_profiles.rs` — load/format/apply proxy profiles
- `database_stats.rs` / `database_report.rs` — SQL builders + fetch
- `resolve.rs` — connection + proxy resolution for tools
- `batch.rs` — sequential / parallel batch runners
- `progress.rs` — CLI stderr / MCP buffered progress

## Install into DBX monorepo

```bash
# requires full checkout with crates/dbx-core
rsync -a crates/dbx-mcp/ /path/to/dbx/crates/dbx-mcp/
rsync -a crates/dbx-cli/ /path/to/dbx/crates/dbx-cli/
# dbx-core failover runtime (copy modules only — see crates/dbx-core/APPLY.md):
cp crates/dbx-core/src/connect_progress.rs /path/to/dbx/crates/dbx-core/src/
cp crates/dbx-core/src/connection.rs /path/to/dbx/crates/dbx-core/src/
cp crates/dbx-core/src/lib.rs /path/to/dbx/crates/dbx-core/src/
cp crates/dbx-core/src/db/proxy_tunnel.rs /path/to/dbx/crates/dbx-core/src/db/
cp crates/dbx-core/src/storage.rs /path/to/dbx/crates/dbx-core/src/
cargo build -p dbx-mcp --release
cargo build -p dbx-cli --release --no-default-features
```

**Do not** expect the old Node `packages/mcp-server/src/index.ts` / `node-core` path to work against packages 0.4.40 (Node patches abandoned).

## Remaining / optional gaps

| Item | Notes |
|------|-------|
| MCP Redis range batch | CLI has ranges; MCP `dbx_execute_redis_command` still single-connection |
| WebBackend query timeout | Local backend honors `timeout_secs`; Web `/api/query/execute` may not forward timeout |
| Remote Web 405 on `connections update` | Needs full monorepo **dbx-web** deploy with `POST /connection/mcp/update` |
| duckdb 1.10504.0 (MSVC 14.51) | Bump in full monorepo `dbx-core` / `src-tauri` Cargo.toml |
| Deeper reuse of upstream catalog stats APIs | Optional optimization |
| Validate against live multi-DB fleets | User-side `cargo build` + smoke |

## Contributing upstream

Intended as a candidate PR to [t8y2/dbx](https://github.com/t8y2/dbx) once validated against real connections.
