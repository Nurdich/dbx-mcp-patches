# Crates

Rust crates for DBX live here.

## Directories

- `dbx-core/` - shared database core, drivers, schema/query logic, import/export, transfer, and plugin support.
- `dbx-cli/` - command-line binary published as `dbx-cli` (avoids colliding with the desktop `dbx` binary from `src-tauri`).
- `dbx-mcp/` - MCP server binary published as `dbx-mcp`.
- `dbx-web/` - the Docker/web backend service binary published as `dbx-web`.

The workspace root is defined in the repository-level `Cargo.toml`.
