# dbx-mcp-patches

Local enhancements for [DBX](https://github.com/t8y2/dbx) **Rust** MCP Server and CLI.

**Upstream:** [t8y2/dbx](https://github.com/t8y2/dbx) — this repo contains **patched crate sources + thin npm launchers**, not the full DBX monorepo (`dbx-core` stays upstream / see `dbx-main-rust`).

## Upstream change (2026-07-19 / v0.5.61+)

| Item | Value |
|------|-------|
| Baseline commit | `fe636d2d` (origin/main @ 2026-07-20) |
| MCP crate | `crates/dbx-mcp` **0.4.38** |
| CLI crate | `crates/dbx-cli` **0.4.38** |
| npm | `@dbx-app/mcp-server` / `@dbx-app/cli` **0.4.38** = thin Node launcher → platform Rust binary |
| Node 0.4.x patches | **Abandoned** — official no longer updates Node tool impl; see [LEGACY.md](./LEGACY.md) |

Release note highlight: **原生 MCP Server / 原生 DBX CLI** — tools run in Rust; npm only resolves `@dbx-app/mcp-<platform>` / `cli-<platform>` binaries.

## Strategy

**Rust-only going forward.** All patch features live in `crates/dbx-mcp` / `crates/dbx-cli` (plus `dbx-core` connect/proxy patches in the full monorepo). Keep npm `packages/*/bin/*.js` as upstream thin launchers only (no Node tool logic).

## Layout

| Path | Role |
|------|------|
| `crates/dbx-mcp` | Patched Rust MCP (tools + stats/report/proxy/list-index) |
| `crates/dbx-cli` | Patched Rust CLI (stats/report/proxies/parallel/short flags/add/remove/redis) |
| `packages/mcp-server` | Upstream npm launcher (spawn Rust binary) |
| `packages/cli` | Upstream npm launcher |
| `LEGACY.md` | Note: Node 0.4.x tree removed / abandoned |
| `UPSTREAM_BASELINE.txt` | Exact upstream SHA |

## Features ported to Rust

- Proxy: inline params + saved tunnel profiles on `dbx_add_connection`
- **Proxy failover group** (ordered try-next, not multi-hop): `proxy_profile_ids` / `1,2,3` / `#1-#3` / repeated `--proxy-profile-name`
- `dbx_list_proxies` / `dbx proxies list`
- `dbx_get_database_stats` / `dbx stats` (catalog `TABLE_ROWS` / estimates, no `COUNT(*)`)
- `dbx_get_database_report` / `dbx report` (tables + comments + indexes; report saves to `{cwd}/reports/`)
- Numeric list IDs + ranges (`1`, `#2`, `1-15`) on MCP + CLI batch tools
- `skip_unsupported` + Skipped vs Failures for batch stats/report
- One-shot `proxy_profile_*` override on stats/report/**query**
- MCP range batch: list_tables / describe / execute_query / schema_context (sequential + progress prepend)
- CLI: `--parallel`/`-P`, short flags, `connections add|remove`, `dbx redis`, stderr progress, soft-fail exit

See [PATCHES.md](./PATCHES.md) and [update_log.md](./update_log.md) for the gap matrix.

## Build / use (you must build — agents do not compile)

Merge these crates into a full [t8y2/dbx](https://github.com/t8y2/dbx) checkout (needs `crates/dbx-core`):

```bash
# from full dbx monorepo
cp -r /path/to/dbx-mcp-patches/crates/dbx-mcp ./crates/
cp -r /path/to/dbx-mcp-patches/crates/dbx-cli ./crates/

cargo build -p dbx-mcp --release
cargo build -p dbx-cli --release --no-default-features
```

Point Cursor / npm launcher at the built binary:

```bash
# Windows example
set DBX_MCP_BINARY=C:\path\to\dbx\target\release\dbx-mcp.exe
# or copy over the platform package bin:
#   node_modules\@dbx-app\mcp-win32-x64\bin\dbx-mcp.exe
```

Install npm launcher (optional, for `npx` / `dbx-mcp-server`):

```bash
npm install -g @dbx-app/mcp-server@0.4.38
# then replace optionalDependency binary with your release build
```

## License

Same as upstream DBX — see [t8y2/dbx LICENSE](https://github.com/t8y2/dbx/blob/main/LICENSE).
