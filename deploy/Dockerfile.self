# WSLC-oriented local image build (native amd64, no BuildKit cache mounts / --platform).
# Build:
#   wslc build -f deploy/Dockerfile.self -t dbx-self:latest .

# Stage 1: Frontend
FROM node:22-slim AS frontend
WORKDIR /app
RUN npm i -g pnpm
COPY package.json pnpm-lock.yaml pnpm-workspace.yaml ./
COPY packages/mongo-shell/ packages/mongo-shell/
RUN pnpm install --frozen-lockfile
COPY apps/desktop/ apps/desktop/
COPY crates/dbx-core/assets/database-drivers.manifest.json crates/dbx-core/assets/database-drivers.manifest.json
RUN pnpm build

# Stage 2: Rust backend (native, no zig cross-compile)
FROM rust:1-bookworm AS backend
WORKDIR /app
RUN apt-get update && apt-get install -y --no-install-recommends \
    build-essential cmake pkg-config perl \
    libfontconfig-dev libfreetype-dev \
    && rm -rf /var/lib/apt/lists/*

ENV CARGO_BUILD_JOBS=2

COPY Cargo.toml Cargo.lock ./
COPY crates/dbx-core/Cargo.toml crates/dbx-core/
COPY crates/dbx-web/Cargo.toml crates/dbx-web/
COPY src-tauri/Cargo.toml src-tauri/
RUN mkdir -p crates/dbx-core/src && echo '' > crates/dbx-core/src/lib.rs \
    && mkdir -p crates/dbx-web/src && echo 'fn main() {}' > crates/dbx-web/src/main.rs \
    && mkdir -p src-tauri/src && echo 'fn main() {}' > src-tauri/src/main.rs && echo 'pub fn run() {}' > src-tauri/src/lib.rs

COPY src-tauri/build.rs src-tauri/
COPY src-tauri/tauri.conf.json src-tauri/

RUN cargo build --release -p dbx-web || true

COPY crates/ crates/
RUN find crates/ -name '*.rs' -exec touch {} +
RUN cargo build --release -p dbx-web \
    && mkdir -p /out \
    && cp /app/target/release/dbx-web /out/dbx-web

# Stage 3: Final image
FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    fontconfig \
    fonts-dejavu-core \
    libfreetype6 \
    libssl3 \
    && rm -rf /var/lib/apt/lists/*
COPY --from=backend /out/dbx-web /usr/local/bin/
COPY --from=frontend /app/dist /app/static
ENV DBX_STATIC_DIR=/app/static
ENV DBX_DATA_DIR=/app/data
EXPOSE 4224
CMD ["dbx-web"]
