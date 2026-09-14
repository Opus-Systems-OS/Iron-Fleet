# Iron-Fleet rig-gpu worker — CUDA runtime image for the self-hosted worker.
#
# Build context is the REPO ROOT (needs the workspace Cargo.toml), so build
# with:   docker build -f worker/Dockerfile .
# This does not run on Railway — it runs on the rig itself, started manually
# or via a systemd unit / docker compose on that machine.

# ---- build ---------------------------------------------------------------
FROM rust:1-bookworm AS build
WORKDIR /src

COPY Cargo.toml Cargo.lock ./
COPY control-plane/Cargo.toml control-plane/Cargo.toml
COPY worker/Cargo.toml worker/Cargo.toml
RUN mkdir -p control-plane/src worker/src \
 && echo 'fn main() {}' > control-plane/src/main.rs \
 && echo 'fn main() {}' > worker/src/main.rs \
 && cargo build --release -p worker \
 && rm -rf control-plane/src worker/src target/release/deps/worker-* target/release/deps/iron_fleet_worker-*

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
