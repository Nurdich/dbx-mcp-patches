# Legacy Node packages

The directories under `legacy-node-packages/` are the **pre-Rust** patches against `@dbx-app/{mcp-server,cli,node-core}@0.4.31`.

Upstream MCP/CLI are now Rust (`crates/dbx-mcp`, `crates/dbx-cli`, npm 0.4.38+ launchers). Do **not** merge these Node packages into modern DBX checkouts expecting them to be the MCP runtime.

Kept only as a reference for feature behavior while finishing the Rust port.
