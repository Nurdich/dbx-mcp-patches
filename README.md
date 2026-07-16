# dbx-mcp-patches

Local enhancements for [DBX](https://github.com/t8y2/dbx) MCP Server, CLI, and node-core.

**Upstream:** [t8y2/dbx](https://github.com/t8y2/dbx) — this repo contains **only the patched packages**, not the full DBX monorepo.

## Quick links

- [PATCHES.md](./PATCHES.md) — what changed vs upstream
- [update_log.md](./update_log.md) — detailed change log (2026-07-16)

## Packages

| Package | Path |
|---------|------|
| MCP Server | `packages/mcp-server` |
| CLI | `packages/cli` |
| node-core | `packages/node-core` |

## Features (2026-07-16)

- Proxy: inline params + saved tunnel profiles (`dbx_list_proxies`)
- Stats: `dbx_get_database_stats` / `dbx stats` (catalog, rows desc)
- Report: `dbx_get_database_report` / `dbx report`
- Numeric list IDs (`#` column, use `1` / `#2` for connections and proxies)
- Streaming output (CLI stderr / MCP tool responses / Web API client stages)

## Install into DBX monorepo

Copy or merge these `packages/*` directories into your [t8y2/dbx](https://github.com/t8y2/dbx) checkout, then:

```bash
pnpm install
pnpm build:packages
```

## License

Same as upstream DBX — see [t8y2/dbx LICENSE](https://github.com/t8y2/dbx/blob/main/LICENSE).
