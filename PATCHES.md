# DBX MCP / CLI Patches (2026-07-16)

This fork branch contains **local enhancements** on top of upstream [t8y2/dbx](https://github.com/t8y2/dbx).

## Upstream baseline

| Item | Value |
|------|-------|
| Upstream repo | https://github.com/t8y2/dbx |
| Base commit | `e226a56` — fix(navigator): preserve table identity when opening objects |
| Patched packages | `packages/mcp-server`, `packages/cli`, `packages/node-core` |

## What we patched (not in upstream)

### MCP Server (`@dbx-app/mcp-server`)

- **`dbx_add_connection`** — inline SOCKS5/HTTP proxy params + saved proxy profile refs (`proxy_profile_id` / `proxy_profile_name`)
- **`dbx_list_proxies`** — list saved proxy tunnel profiles from DBX Settings > Tunnels
- **`dbx_get_database_stats`** — catalog-based table/database stats (no `COUNT(*)`)
- **`dbx_get_database_report`** — full report: summary, tables (sorted by rows desc), column comments, indexes
- **Numeric list IDs** — `#` column in lists; use `1` / `#2` instead of UUID/name for connections and proxy profiles

### CLI (`@dbx-app/cli`)

- **`dbx connections add`** — proxy flags aligned with MCP
- **`dbx proxies list`** — mirrors `dbx_list_proxies`
- **`dbx stats`** — mirrors `dbx_get_database_stats`
- **`dbx report`** — mirrors `dbx_get_database_report`
- Numeric connection/proxy references in all relevant commands

### node-core (`@dbx-app/node-core`)

- `tunnel-profiles.ts` — load tunnel profiles (desktop + web backend)
- `database-stats.ts` — `fetchDatabaseStats`, row-desc sort, system catalog exclusion
- `database-report.ts` — `fetchDatabaseReport`, index/column comment SQL builders
- `list-index.ts` — `parseListIndex`, index-based connection/proxy resolution
- `connections.ts` / `backend.ts` / `web-backend.ts` — `profile_id` + `loadTunnelProfiles`

## Installed copies (for runtime, not in this git tree)

`dist/` is gitignored upstream. After building or copying dist manually:

| Location | Purpose |
|----------|---------|
| `C:\usr\local\node_modules\@dbx-app\mcp-server` | Cursor MCP server |
| `G:\usr\local\node_modules\@dbx-app\cli` | `dbx` CLI |

See **`update_log.md`** for full change history, verification commands, and known limits.

## Build / install

```bash
pnpm install
pnpm build:packages   # produces packages/*/dist
```

Or link locally:

```bash
npm link -C packages/node-core
npm link -C packages/mcp-server
npm link -C packages/cli
```

## Contributing upstream

These changes are intended as a candidate PR to [t8y2/dbx](https://github.com/t8y2/dbx). Open an issue or PR there if you want them merged officially.
