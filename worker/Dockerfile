# Iron-Fleet rig-gpu worker — CUDA runtime image for the self-hosted worker.
#
# Build context is the REPO ROOT (needs the workspace Cargo.toml), so build
# with:   docker build -f worker/Dockerfile .
# This does not run on the droplet — it runs on the rig itself, started manually
# or via a systemd unit / docker compose on that machine.

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
 && cargo build --release -p worker \
 # Only worker's own dummy src is replaced next — the other members' dummies
 # must survive, since cargo re-resolves the whole workspace on every build
 # and their real sources are never copied into this image at all.
 && rm -rf worker/src target/release/deps/worker-* target/release/deps/iron_fleet_worker-*

COPY worker/src worker/src
RUN cargo build --release -p worker

# ---- runtime -------------------------------------------------------------
# nvidia/cuda runtime (not devel — the worker doesn't compile CUDA, it runs
# tool calls that use it) with the driver's userspace libraries; the host's
# NVIDIA driver + nvidia-container-toolkit is what actually exposes the GPU
# (`docker run --gpus all`).
FROM nvidia/cuda:12.6.0-runtime-ubuntu22.04
RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates bash \
 && rm -rf /var/lib/apt/lists/*

WORKDIR /app
COPY --from=build /src/target/release/iron-fleet-worker /usr/local/bin/iron-fleet-worker

ENV WORKER_WORKDIR=/app/work
VOLUME ["/app/work"]
CMD ["iron-fleet-worker", "run"]
