# Iron-Fleet control plane — Railway deployment image.
#
# Build context is the REPO ROOT (the workspace Cargo.toml and agents/ are
# needed), so build with:   docker build -f control-plane/Dockerfile .
# Railway does this for you when the service's Dockerfile path is
# control-plane/Dockerfile and the root directory is the repo root.

# ---- build ---------------------------------------------------------------
FROM rust:1-bookworm AS build
WORKDIR /src

# Cache dependency compilation separately from source changes.
COPY Cargo.toml Cargo.lock ./
COPY control-plane/Cargo.toml control-plane/Cargo.toml
RUN mkdir -p control-plane/src && echo 'fn main() {}' > control-plane/src/main.rs \
 && cargo build --release -p control-plane \
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

# Runs as root on purpose: Railway volumes are mounted root-owned and Railway's
# documented workaround for non-root images is RAILWAY_RUN_UID=0 anyway. The
# process only ever writes to the volume path.
ENV AGENTS_DIR=/app/agents \
    RUST_LOG=info,tower_http=info
# Railway injects PORT; 8080 is the local default.
EXPOSE 8080
CMD ["control-plane", "serve"]
