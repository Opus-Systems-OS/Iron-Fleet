# Iron-Fleet mcp-fleet — deployment image.
#
# Build context is the REPO ROOT (needs the workspace Cargo.toml), so build
# with:   docker build -f mcp-fleet/Dockerfile .
# Where this actually runs is an open decision (CLAUDE.md documents only
# control-plane's Railway hosting so far) — it needs the same thing
# control-plane needed: a public HTTPS endpoint, since Anthropic's Managed
# Agents session reaches it as a remote `mcp_servers` URL.

# ---- build ---------------------------------------------------------------
FROM rust:1-bookworm AS build
WORKDIR /src

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
 && cargo build --release -p mcp-fleet \
 # Only mcp-fleet's own dummy src is replaced next — the other members'
 # dummies must survive, since cargo re-resolves the whole workspace on every
 # build and their real sources are never copied into this image at all.
 && rm -rf mcp-fleet/src target/release/deps/mcp_fleet-*

COPY mcp-fleet/src mcp-fleet/src
RUN cargo build --release -p mcp-fleet

# ---- runtime -------------------------------------------------------------
FROM debian:bookworm-slim
RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates \
 && rm -rf /var/lib/apt/lists/*

WORKDIR /app
COPY --from=build /src/target/release/mcp-fleet /usr/local/bin/mcp-fleet

ENV RUST_LOG=info,tower_http=info
EXPOSE 8090
CMD ["mcp-fleet"]
