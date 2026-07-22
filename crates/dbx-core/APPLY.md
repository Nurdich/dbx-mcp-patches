# Applying `dbx-core` failover / progress / MCP-update patches

These files are a **subset** of upstream [`t8y2/dbx`](https://github.com/t8y2/dbx) `crates/dbx-core`, captured from local `dbx-main-rust` working tree (baseline `1b399a58`) with multi-proxy **failover** + connect **progress** runtime + MCP/CLI **connection update** storage API.

This repo is **not** a full monorepo. Copy these modules into a complete DBX checkout that already has `crates/dbx-core`.

## Files

| Patch path | Upstream destination |
|------------|----------------------|
| `src/connect_progress.rs` | `crates/dbx-core/src/connect_progress.rs` (**new**) |
| `src/connection.rs` | `crates/dbx-core/src/connection.rs` (replace) |
| `src/lib.rs` | `crates/dbx-core/src/lib.rs` (replace; adds `mod connect_progress`) |
| `src/db/proxy_tunnel.rs` | `crates/dbx-core/src/db/proxy_tunnel.rs` (replace) |
| `src/storage.rs` | `crates/dbx-core/src/storage.rs` (replace; adds `update_connection_for_mcp`) |

Optional unified diff (failover only, may lag storage): [`../../patches/dbx-core-failover.patch`](../../patches/dbx-core-failover.patch). Prefer file copy.

## Apply (preferred: copy sources)

From a full DBX monorepo root (must already contain upstream `crates/dbx-core`):

```bash
# Windows (PowerShell)
$PATCHES = "G:\rust\dbx-mcp-patches"
Copy-Item -Force "$PATCHES\crates\dbx-core\src\connect_progress.rs" .\crates\dbx-core\src\
Copy-Item -Force "$PATCHES\crates\dbx-core\src\connection.rs" .\crates\dbx-core\src\
Copy-Item -Force "$PATCHES\crates\dbx-core\src\lib.rs" .\crates\dbx-core\src\
Copy-Item -Force "$PATCHES\crates\dbx-core\src\db\proxy_tunnel.rs" .\crates\dbx-core\src\db\
Copy-Item -Force "$PATCHES\crates\dbx-core\src\storage.rs" .\crates\dbx-core\src\
```

```bash
# Unix
PATCHES=/path/to/dbx-mcp-patches
cp "$PATCHES/crates/dbx-core/src/connect_progress.rs" ./crates/dbx-core/src/
cp "$PATCHES/crates/dbx-core/src/connection.rs" ./crates/dbx-core/src/
cp "$PATCHES/crates/dbx-core/src/lib.rs" ./crates/dbx-core/src/
cp "$PATCHES/crates/dbx-core/src/db/proxy_tunnel.rs" ./crates/dbx-core/src/db/
cp "$PATCHES/crates/dbx-core/src/storage.rs" ./crates/dbx-core/src/
```

Or with rsync alongside MCP/CLI:

```bash
rsync -a /path/to/dbx-mcp-patches/crates/dbx-mcp/ ./crates/dbx-mcp/
rsync -a /path/to/dbx-mcp-patches/crates/dbx-cli/ ./crates/dbx-cli/
# only the modules above — do not wipe the rest of dbx-core
cp /path/to/dbx-mcp-patches/crates/dbx-core/src/connect_progress.rs ./crates/dbx-core/src/
cp /path/to/dbx-mcp-patches/crates/dbx-core/src/connection.rs ./crates/dbx-core/src/
cp /path/to/dbx-mcp-patches/crates/dbx-core/src/lib.rs ./crates/dbx-core/src/
cp /path/to/dbx-mcp-patches/crates/dbx-core/src/db/proxy_tunnel.rs ./crates/dbx-core/src/db/
cp /path/to/dbx-mcp-patches/crates/dbx-core/src/storage.rs ./crates/dbx-core/src/
```

## Apply (optional: git patch)

```bash
cd /path/to/full-dbx-monorepo
git apply /path/to/dbx-mcp-patches/patches/dbx-core-failover.patch
```

If line endings / baseline drift, prefer the file-copy method above. The optional `.patch` may not include `storage.rs` — copy that file separately.

## What this enables

- Ordered proxy **failover group** (try next on failure; not multi-hop chain)
- `connect_progress` hook so MCP/CLI can surface `[dbx] Trying proxy…` lines
- Eager `verify_proxy_connect` for pure-proxy stacks during failover
- `Storage::update_connection_for_mcp` for CLI/MCP connection update + winner writeback

## Related monorepo-only (not in this patches repo)

- **duckdb** bump to `1.10504.0` in full `crates/dbx-core/Cargo.toml` / `src-tauri` (MSVC 14.51) — apply in monorepo, not shipped here
- **`dbx-web`** `POST /connection/mcp/update` — required to clear remote Web 405; deploy full monorepo `dbx-web`

## Build (you run; agents do not compile)

```bash
cargo build -p dbx-mcp --release
cargo build -p dbx-cli --release --no-default-features
```

Requires the patched `crates/dbx-mcp` / `crates/dbx-cli` from this same patches repo.
