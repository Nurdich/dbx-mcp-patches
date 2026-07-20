# DBX MCP / CLI Patches (Rust era — 2026-07-21)

Local enhancements on top of upstream [t8y2/dbx](https://github.com/t8y2/dbx) **Rust** MCP/CLI.

## Upstream baseline

| Item | Value |
|------|-------|
| Upstream repo | https://github.com/t8y2/dbx |
| Base commit | `fe636d2d` — chore(jdbc): bump plugin version (main @ 2026-07-20) |
| Nearby tags | `v0.5.61` / `v0.5.62`, packages `0.4.38` |
| MCP | `crates/dbx-mcp` 0.4.38 |
| CLI | `crates/dbx-cli` 0.4.38 |
| npm | Thin launchers only (`packages/mcp-server`, `packages/cli`) |
| Node 0.4.x | **Abandoned** — see [LEGACY.md](./LEGACY.md); tree `legacy-node-packages/` removed |

Previous Node baseline (`packages-v0.4.31` / `5206750`) is no longer maintained; all features live in Rust crates (+ `dbx-core` patches).

## What we patched (not in upstream Rust)

### MCP (`crates/dbx-mcp`)

| Feature | Status |
|---------|--------|
| `dbx_list_proxies` | **Ported** |
| `dbx_get_database_stats` / `dbx_get_database_report` | **Ported** |
| Numeric `#` / ranges on connection selectors | **Ported** |
| `#` column in `dbx_list_connections` | **Ported** |
| Inline proxy + `proxy_profile_*` on `dbx_add_connection` | **Ported** |
| Multi-proxy **failover group** (try-next, not chain) | **Ported** (MCP + CLI + `dbx-core` connect) |
| One-shot `proxy_profile_*` on stats/report/**query** | **Ported** |
| Batch ranges on list_tables / describe / query / schema_context / stats / report | **Ported** (sequential) |
| `skip_unsupported` + Skipped vs Failures | **Ported** |
| Progress prepend in tool text (`DBX_MCP_QUIET` / `DBX_MCP_VERBOSE`) | **Ported** |
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
# failover runtime also needs these upstream-core edits (already applied in dbx-main-rust):
#   crates/dbx-core/src/connect_progress.rs
#   crates/dbx-core/src/connection.rs (connection_host_port failover)
#   crates/dbx-core/src/db/proxy_tunnel.rs (verify_proxy_connect)
cargo build -p dbx-mcp --release
cargo build -p dbx-cli --release --no-default-features
```

**Do not** expect the old Node `packages/mcp-server/src/index.ts` / `node-core` path to work against packages ≥0.4.38 (Node patches abandoned).

## Remaining / optional gaps

| Item | Notes |
|------|-------|
| MCP Redis range batch | CLI has ranges; MCP `dbx_execute_redis_command` still single-connection |
| Deeper reuse of upstream catalog stats APIs | Optional optimization |
| Validate against live multi-DB fleets | User-side `cargo build` + smoke |

## Contributing upstream

Intended as a candidate PR to [t8y2/dbx](https://github.com/t8y2/dbx) once validated against real connections.
