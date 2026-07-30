# syntax=docker/dockerfile:1
#
# Cached dependency builder for the fast patch workflow.
# Build ONCE (or whenever Cargo.toml / Cargo.lock / package manifests change), then
# reuse it from deploy/Dockerfile.patch so each patch build only recompiles app code.
#
#   wslc build -f deploy/Dockerfile.builder -t dbx-web-builder:base .
#
# This image holds the Rust toolchain + ALL dbx-web dependencies pre-compiled against
# dummy sources. It mirrors the upstream deploy/Dockerfile dependency-precompile stage
# but is native amd64 (no zigbuild) to match the WSLC self-build environment.
#
# IMPORTANT: copy EVERY workspace member's Cargo.toml. The earlier deploy/Dockerfile.self
# only copied 3 of 5 manifests, so `cargo build` could not load the workspace and the
# dependency cache silently never populated — every build recompiled everything (hours).
# This builder copies all 5 so the cache actually works.

FROM rust:1-bookworm
WORKDIR /app

RUN apt-get update && apt-get install -y --no-install-recommends \
        build-essential cmake pkg-config perl \
        libfontconfig-dev libfreetype-dev \
    && rm -rf /var/lib/apt/lists/*

ENV CARGO_BUILD_JOBS=2

# Dependency manifests only (no real src yet). All five workspace members must be
# present or `cargo` refuses to load the workspace and the dependency cache is lost.
COPY Cargo.toml Cargo.lock ./
COPY crates/dbx-core/Cargo.toml crates/dbx-core/
COPY crates/dbx-web/Cargo.toml crates/dbx-web/
COPY crates/dbx-mcp/Cargo.toml crates/dbx-mcp/
COPY crates/dbx-cli/Cargo.toml crates/dbx-cli/
COPY src-tauri/Cargo.toml src-tauri/
COPY vendor/ctor/ vendor/ctor/

# Dummy sources so cargo can pre-compile every dependency without the real app code.
RUN mkdir -p crates/dbx-core/src && echo '' > crates/dbx-core/src/lib.rs \
    && mkdir -p crates/dbx-web/src && echo 'fn main() {}' > crates/dbx-web/src/main.rs \
    && mkdir -p crates/dbx-mcp/src && echo '' > crates/dbx-mcp/src/lib.rs && echo 'fn main() {}' > crates/dbx-mcp/src/main.rs \
    && mkdir -p crates/dbx-cli/src && echo 'fn main() {}' > crates/dbx-cli/src/main.rs \
    && mkdir -p src-tauri/src && echo 'fn main() {}' > src-tauri/src/main.rs && echo 'pub fn run() {}' > src-tauri/src/lib.rs

COPY src-tauri/build.rs src-tauri/
COPY src-tauri/tauri.conf.json src-tauri/

# Pre-compile dependencies. `|| true` tolerates the dummy app crates failing to link;
# the dependency artifacts remain cached in target/ for the patch stage to reuse.
RUN cargo build --release -p dbx-web || true
