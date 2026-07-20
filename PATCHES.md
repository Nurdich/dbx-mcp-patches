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

Previous Node baseline (`packages-v0.4.31` / `5206750`) is archived in `legacy-node-packages/`.

## What we patched (not in upstream Rust)

### MCP (`crates/dbx-mcp`)

| Feature | Status |
|---------|--------|
| `dbx_list_proxies` | **Ported** |
| `dbx_get_database_stats` | **Ported** (catalog estimates; Redis/Mongo special-cased) |
| `dbx_get_database_report` | **Ported** |
| Numeric `#` / ranges on connection selectors | **Ported** |
| `#` column in `dbx_list_connections` | **Ported** |
| Inline proxy + `proxy_profile_*` on `dbx_add_connection` | **Ported** |
| One-shot `proxy_profile_*` on stats/report | **Ported** |
| Batch stats/report + `skip_unsupported` | **Ported** |
| Proxy override on `dbx_execute_query` | **Pending** |
| Streaming connection progress in tool text | **Pending** (catalog path is single-shot) |
| Parallel batch (MCP) | N/A (CLI-only historically) |

### CLI (`crates/dbx-cli`)

| Feature | Status |
|---------|--------|
| `dbx proxies list` | **Ported** |
| `dbx stats` / `dbx report` | **Ported** |
| List index / ranges | **Ported** |
| Report save to `{cwd}/reports/` | **Ported** (`-n` / `--no-save`) |
| `#` column in `connections list` | **Ported** |
| `--parallel` / short-flag parity / `connections add` proxy flags | **Pending** |
| `connections remove` / `dbx redis` | **Pending** (upstream CLI still narrower) |

### Shared modules (inside `dbx-mcp` crate)

- `list_index.rs` — `1` / `#2` / `1-15` / `1..15`
- `tunnel_profiles.rs` — load/format/apply proxy profiles
- `database_stats.rs` / `database_report.rs` — SQL builders + fetch
- `resolve.rs` — connection + proxy resolution for tools

## Install into DBX monorepo

```bash
# requires full checkout with crates/dbx-core
rsync -a crates/dbx-mcp/ /path/to/dbx/crates/dbx-mcp/
rsync -a crates/dbx-cli/ /path/to/dbx/crates/dbx-cli/
cargo build -p dbx-mcp --release
cargo build -p dbx-cli --release --no-default-features
```

**Do not** expect the old Node `packages/mcp-server/src/index.ts` path to work against packages ≥0.4.38.

## Contributing upstream

Intended as a candidate PR to [t8y2/dbx](https://github.com/t8y2/dbx) once validated against real connections.
