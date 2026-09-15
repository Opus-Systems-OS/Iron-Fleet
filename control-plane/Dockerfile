# Iron-Fleet control plane — deployment image.
#
# Build context is the REPO ROOT (the workspace Cargo.toml and agents/ are
# needed), so build with:   docker build -f control-plane/Dockerfile .
# .github/workflows/images.yml does this on every push to main and pushes
# the result to GHCR; the droplet only ever pulls.

# ---- build ---------------------------------------------------------------
FROM rust:1-bookworm AS build
WORKDIR /src

# Cache dependency compilation separately from source changes.
COPY Cargo.toml Cargo.lock ./
# Every workspace member's Cargo.toml has to be present for cargo to resolve
# the workspace at all, even ones this image never builds — cargo checks
# each member's [lib]/[[bin]] source paths exist while loading the manifest.
COPY control-plane/Cargo.toml control-plane/Cargo.toml
COPY worker/Cargo.toml worker/Cargo.toml
COPY app/src-tauri/Cargo.toml app/src-tauri/Cargo.toml
COPY mcp-fleet/Cargo.toml mcp-fleet/Cargo.toml
RUN mkdir -p control-plane/src worker/src app/src-tauri/src mcp-fleet/src \
 && echo 'fn main() {}' > control-plane/src/main.rs \
 && echo 'fn main() {}' > worker/src/main.rs \
 && echo 'fn main() {}' > app/src-tauri/src/main.rs \
 && echo '' > app/src-tauri/src/lib.rs \
 && echo 'fn main() {}' > mcp-fleet/src/main.rs \
 && cargo build --release -p control-plane \
 # Only control-plane's own dummy src is replaced next — the other members'
 # dummies must survive, since cargo re-resolves the whole workspace on every
 # build and their real sources are never copied into this image at all.
 && rm -rf control-plane/src target/release/deps/control_plane-*

COPY control-plane/src control-plane/src
RUN cargo build --release -p control-plane

# ---- runtime -------------------------------------------------------------
FROM debian:bookworm-slim
RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates \
 && rm -rf /var/lib/apt/lists/*

WORKDIR /app
COPY --from=build /src/target/release/control-plane /usr/local/bin/control-plane
# The committed fleet definition ships in the image; boot sync reads it.
COPY agents /app/agents

# Runs as root on purpose: the Docker named volume for /data is root-owned.
# The process only ever writes to the volume path.
ENV AGENTS_DIR=/app/agents \
    RUST_LOG=info,tower_http=info
# PORT is set by the compose file; 8080 is the local default.
EXPOSE 8080
CMD ["control-plane", "serve"]
