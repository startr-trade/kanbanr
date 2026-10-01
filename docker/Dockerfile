# syntax=docker/dockerfile:1
# One image, one binary: `kanbanr` runs the view daemon (`kanbanr serve`) which serves the built
# React SPA + read API + live SSE over the mounted data folder. Docker is OPTIONAL — the same
# binary runs natively with no container. Build context must be the repo root:
#     docker build -f docker/Dockerfile -t kanbanr \
#       --build-arg KANBANR_GIT_SHA="$(git rev-parse --short=12 HEAD)" \
#       --build-arg KANBANR_BUILD_DATE="$(git log -1 --format=%cd --date=short)" .

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
# The monitor is baked INTO the binary (FEAT-084), so the built SPA has to be present for this
# build rather than copied beside the binary afterwards. One mechanism everywhere: the image then
# exercises the same embedded path a downloaded release binary takes, instead of being the one
# deployment where a stale --ui-dir could still work and hide a broken embed.
COPY --from=web /web/dist /web/dist
ENV KANBANR_UI_DIST=/web/dist
# The build context has no .git (.dockerignore), so the commit and its date come in as arguments,
# or `--version` says `unknown` (FEAT-144). Declared here, after the cooked dependencies, so a new
# commit does not invalidate that layer.
ARG KANBANR_GIT_SHA=""
ARG KANBANR_BUILD_DATE=""
RUN KANBANR_GIT_SHA="$KANBANR_GIT_SHA" KANBANR_BUILD_DATE="$KANBANR_BUILD_DATE" \
    cargo build --release -p kanbanr-cli

# ---- Stage 3: slim runtime — the one binary, in serve mode ----
FROM debian:bookworm-slim
# Only CA certs (so any git HTTPS sync verifies TLS). libgit2 is linked statically — no git needed.
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*
WORKDIR /app
# Just the binary: the monitor is inside it.
COPY --from=api /api/target/release/kanbanr /usr/local/bin/kanbanr

ENV KANBANR_DATA_DIR=/data \
    KANBANR_BIND=0.0.0.0:8080
EXPOSE 8080
VOLUME ["/data"]
# Not root (FEAT-132). The daemon only reads the board, and a board's files are world-readable as git
# writes them, so an unprivileged user needs nothing more; a container escape then lands without root.
RUN useradd --system --uid 10001 --no-create-home --shell /usr/sbin/nologin kanbanr
USER 10001

# Healthy means the daemon answers /healthz. No curl in a slim image, so bash's /dev/tcp asks.
HEALTHCHECK --interval=30s --timeout=5s --start-period=10s --retries=3 \
    CMD ["bash", "-c", "exec 3<>/dev/tcp/127.0.0.1/8080 && printf 'GET /healthz HTTP/1.0\\r\\n\\r\\n' >&3 && head -1 <&3 | grep -q ' 200 '"]

# The view daemon is read-only and unauthenticated; expose it beyond localhost only behind a
# reverse proxy you control.
CMD ["kanbanr", "serve"]
