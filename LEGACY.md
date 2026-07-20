# Legacy Node packages — abandoned

**Status (2026-07-21):** Node 0.4.x MCP/CLI patches are **abandoned**. Upstream no longer maintains the Node `@dbx-app/node-core` tool implementation; official npm packages are thin launchers that spawn platform Rust binaries only.

| Era | Location | Notes |
|-----|----------|-------|
| Active | `crates/dbx-mcp`, `crates/dbx-cli` (+ `dbx-core` patches in full monorepo / `dbx-main-rust`) | All features live here |
| npm (kept) | `packages/mcp-server`, `packages/cli` | Upstream thin launchers only — no tool logic |
| Removed | `legacy-node-packages/` (was ~56MB Node 0.4.31 tree) | Deleted from this repo; history remains in git |

Do **not** expect old Node `packages/mcp-server/src/index.ts` / `node-core` patches to work against packages ≥0.4.38.
