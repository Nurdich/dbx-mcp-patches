# Fast patch build for the DBX web API image

This workflow builds a **locally-patched `dbx-web` image in minutes** instead of hours,
by reusing a one-time cached dependency builder and overlaying the patched binary +
frontend onto the official `t8y2/dbx:latest` runtime image.

It does **not** modify the existing `deploy/Dockerfile` or `deploy/Dockerfile.self` —
those remain the full from-source builds.

## Why the old self-build was slow

`deploy/Dockerfile.self` pre-compiles dependencies with dummy sources (`cargo build || true`),
but it only copies **3 of 5** workspace member `Cargo.toml` files (it omits
`crates/dbx-mcp` and `crates/dbx-cli`). With missing workspace members, `cargo` cannot
load the workspace, the dependency build fails silently, and **every build recompiles
all dependencies from scratch** — that is the hours-long cost.

`deploy/Dockerfile.builder` (below) copies **all 5** member manifests, so the dependency
cache actually populates and survives as a tagged image (`dbx-web-builder:base`) that
`wslc build` cannot prune away.

## Files

| File | Role |
|---|---|
| `Dockerfile.builder` | One-time cached dependency builder → `dbx-web-builder:base` |
| `Dockerfile.patch` | Per-patch build: recompile changed Rust + frontend, overlay onto `t8y2/dbx:latest` → `dbx-self:latest` |
| `docker-compose.patch.yml` | Compose wrapper mirroring `docker-compose.self.yml` |

## Workflow

```bash
# 1) One-time (or when Cargo.toml / Cargo.lock change): build the cached builder.
#    Hours, but only once. The tag survives across wslc builds.
wslc build -f deploy/Dockerfile.builder -t dbx-web-builder:base .

# 2) After each local patch change: fast patch build.
wslc build -f deploy/Dockerfile.patch -t dbx-self:latest .
# or
wslc-compose -f deploy/docker-compose.patch.yml build
wslc-compose -f deploy/docker-compose.patch.yml up -d
```

## Expected speed

| Change | Rebuild cost |
|---|---|
| Only frontend TS | `pnpm build` (~1–2 min) + cargo no-op → ~2 min |
| A few Rust crates in dbx-web / dbx-core | recompile affected crates (~3–8 min) |
| `Cargo.toml` / `Cargo.lock` | builder cache invalidates → rebuild `Dockerfile.builder` (hours, rare) |

## How it works

- `Dockerfile.builder` pre-compiles every dbx-web dependency against dummy sources and
  tags the result as `dbx-web-builder:base`. `target/` and the git/registry caches live
  inside that image, so they persist as long as the image exists.
- `Dockerfile.patch` starts `FROM dbx-web-builder:base`, copies the **real** local-patched
  source over the dummy sources, touches the `.rs` files (so cargo notices the change),
  and runs `cargo build --release -p dbx-web`. Cargo recompiles only the changed
  application crates — dependencies are reused from the builder.
- The final stage `FROM t8y2/dbx:latest` (the official runtime image, which already has
  debian + libssl3 + fonts) overwrites `/usr/local/bin/dbx-web` and `/app/static` with the
  patched binary and frontend bundle.

## CI: GitHub Actions (auto-sync + auto-build)

`.github/workflows/patch-build.yml` automates the whole cycle in CI (lives on the
`local-patches` branch of the patches repo):

1. **sync** — merges upstream `t8y2/dbx:main` into `local-patches` and pushes (the
   "auto-patch"). On merge conflicts it fails fast for manual resolution — CI does not
   guess.
2. **build-desktop** — `pnpm tauri build` on `windows-2022` → NSIS installer + portable exe,
   published to the rolling GitHub Release `patch-latest` (and workflow artifacts).
3. **build-image** — `docker build deploy/Dockerfile` (multi-arch, gha-cached) → pushes
   `ghcr.io/<owner>/dbx:{latest,patch,sha-<short>}`.

Triggers:
- `schedule: '0 8 * * *'` (daily) — runs sync, then builds only if upstream had changes.
- `workflow_dispatch` — manual; `skip_sync` input builds the current tip without syncing.
- `push` to `local-patches` — builds your pushed patches (sync skipped).

> **Set the patches repo's default branch to `local-patches`** — GitHub only fires
> `schedule` on the default branch. Manual/push triggers work regardless.

Optional secret: `TAURI_SIGNING_PRIVATE_KEY_BASE64` (+ `..._PASSWORD`) for updater
signatures. Without it the NSIS installer still builds; only the auto-update signature is
skipped.

This CI path uses the official `deploy/Dockerfile` + GitHub Actions cache (no local
builder image needed), so it stays fast after the first run. The local `Dockerfile.patch`
workflow above is for ad-hoc `wslc` builds on your machine.

## Notes / limits
- Target architecture is **native amd64** (matches `Dockerfile.self`). arm64 would need
  the zigbuild cross-compile path from `deploy/Dockerfile`.
- The runtime base is pulled with the official `pull_policy: always`, so runtime library
  updates track upstream automatically.
- `dbx-mcp` is **not** part of this image (dbx-web does not depend on dbx-mcp). The MCP
  server ships as a separate npm package; its fast-build is out of scope here.
- The frontend `pnpm install` runs on every patch build (minutes, not hours). If that
  ever matters, add a `dbx-web-frontend-deps:base` image caching the pnpm store — not
  needed for v1.

## Troubleshooting

- **`cargo build` in the patch stage recompiles everything** → the builder image is stale
  or missing. Re-run step 1 (`wslc build -f deploy/Dockerfile.builder ...`).
- **`unable to prepare context: path ... not found`** → build from the repo root
  (`context: ..` in compose, or `.` with `-f deploy/Dockerfile.patch`).
- **binary segfaults at runtime** → you likely built on a different glibc/arch than the
  official `t8y2/dbx` base. Stay on native amd64 bookworm (the builder is `rust:1-bookworm`,
  matching `debian:bookworm-slim`).
