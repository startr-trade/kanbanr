# syntax=docker/dockerfile:1
# One image, one binary: `kanbanr` runs the view daemon (`kanbanr serve`) which serves the built
# React SPA + read API + live SSE over the mounted data folder. Docker is OPTIONAL — the same
# binary runs natively with no container. Build context must be the repo root:
#     docker build -f docker/Dockerfile -t kanbanr .

# ---- Stage 1: build the web SPA ----
FROM node:22-slim AS web
WORKDIR /web
COPY web/package.json web/package-lock.json* ./
RUN npm install
COPY web/ ./
RUN npm run build      # -> /web/dist

# ---- Stage 2: build the single `kanbanr` binary (cargo-chef for cached dependency layers) ----
# cargo-chef separates DEPENDENCY compilation from APP compilation. The dependency tree — whose
# slow part is the vendored libgit2 + OpenSSL compiled from source — is built once into a layer
# keyed by `recipe.json` (distilled from the Cargo manifests + lockfile). Editing application source
# then NO LONGER recompiles that whole tree: Docker reuses the cached `cook` layer and only the thin
# kanbanr crates rebuild, cutting most image rebuilds from minutes to seconds.
#
# Pin the base to the SAME Debian suite as the runtime stage below (bookworm). The bare
# `rust:1.98-slim` tag floats to the latest Debian (trixie, glibc 2.39); linking against that and
# running on bookworm (glibc 2.36) fails at startup with
#   "libc.so.6: version `GLIBC_2.39' not found (required by kanbanr)".
# Keeping both stages on bookworm makes the binary link glibc 2.36 and run on bookworm and newer.
FROM rust:1.98-slim-bookworm AS chef
RUN cargo install cargo-chef --locked
WORKDIR /api

# Planner: distill the dependency graph into recipe.json (reads manifests/source; compiles nothing).
FROM chef AS planner
COPY api/ ./
RUN cargo chef prepare --recipe-path recipe.json

# Builder (named `api` so the runtime COPY below is unchanged): cook deps into a cached layer, then
# compile the app against them.
FROM chef AS api
# Build-only toolchain for the vendored libgit2 + OpenSSL (compiled from source and statically
# linked, so the RUNTIME image needs no git/libgit2/openssl packages). None of this ships.
RUN apt-get update && apt-get install -y --no-install-recommends \
        cmake make perl gcc libc6-dev \
    && rm -rf /var/lib/apt/lists/*
# Cook the dependencies. This is the expensive (vendored-C) step and its layer is cached until the
# Cargo manifests/lockfile change — source-only edits skip it entirely.
COPY --from=planner /api/recipe.json recipe.json
RUN cargo chef cook --release --recipe-path recipe.json
# Now the real source; with deps already compiled in ./target, only the kanbanr crates rebuild.
COPY api/ ./
RUN cargo build --release -p kanbanr-cli

# ---- Stage 3: slim runtime — the one binary, in serve mode ----
FROM debian:bookworm-slim
# Only CA certs (so any git HTTPS sync verifies TLS). libgit2 is linked statically — no git needed.
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*
WORKDIR /app
COPY --from=api /api/target/release/kanbanr /usr/local/bin/kanbanr
COPY --from=web /web/dist /app/web

ENV KANBANR_DATA_DIR=/data \
    KANBANR_UI_DIR=/app/web \
    KANBANR_BIND=0.0.0.0:8080
EXPOSE 8080
VOLUME ["/data"]
# The view daemon is read-only and unauthenticated; expose it beyond localhost only behind a
# reverse proxy you control.
CMD ["kanbanr", "serve"]
