# Iron-Fleet rig-gpu worker — Anthropic's SDK EnvironmentWorker on the CUDA
# runtime image. See worker/sdk/README.md for why this exists next to the Rust
# crate and how to run it.
#
# Build context is this directory:   docker build -t iron-fleet-worker-sdk worker/sdk
# The image never contains a credential; ANTHROPIC_ENVIRONMENT_ID / _KEY come
# from the compose env_file at run time.

# nvidia/cuda runtime (not devel — tool calls that need nvcc can install it in
# the session). The host driver + Docker Desktop's WSL2 GPU passthrough is what
# exposes the 5070 (`gpus: all` in docker-compose.yml).
FROM nvidia/cuda:12.6.0-runtime-ubuntu22.04

ENV DEBIAN_FRONTEND=noninteractive \
    PIP_NO_CACHE_DIR=1 \
    PIP_DISABLE_PIP_VERSION_CHECK=1 \
    PYTHONUNBUFFERED=1

# The SDK worker requires /bin/bash at that exact path (the `bash` tool owns a
# persistent /bin/bash subprocess); ubuntu:22.04 ships python3.10, which is the
# SDK's floor. curl/git/jq are for the agent's own tool calls.
RUN apt-get update \
 && apt-get install -y --no-install-recommends \
      bash ca-certificates curl git jq python3 python3-pip python3-venv \
 && rm -rf /var/lib/apt/lists/*

# Pinned: the self-hosted worker (`anthropic.lib.environments`) landed in the
# 1.x line; bump deliberately, and re-run the smoke check in the README.
RUN python3 -m pip install --no-cache-dir "anthropic==1.6.0"

# Clipper toolchain (agents/clipper.json + the clipper skill): ffmpeg to cut
# and encode (h264_nvenc when the driver exposes the encoder, libx264 when it
# does not), rclone to put the finished clip in the public clips bucket, Node
# for Buffer's CLI. yt-dlp is deliberately unpinned — YouTube breaks extractors
# every few weeks and a pinned copy quietly stops working; rebuild when it does.
RUN apt-get update \
 && apt-get install -y --no-install-recommends ffmpeg openssl unzip \
 && rm -rf /var/lib/apt/lists/*

# rclone from upstream, not apt: jammy ships 1.53, which predates R2 support
# (`provider = Cloudflare` arrived in 1.59) and would fail on the clips bucket.
RUN curl -fsSLO https://downloads.rclone.org/v1.75.1/rclone-v1.75.1-linux-amd64.zip \
 && unzip -q rclone-v1.75.1-linux-amd64.zip \
 && install -m 755 rclone-v1.75.1-linux-amd64/rclone /usr/local/bin/rclone \
 && rm -rf rclone-v1.75.1-linux-amd64 rclone-v1.75.1-linux-amd64.zip

RUN python3 -m pip install --no-cache-dir yt-dlp

# Node from NodeSource: jammy ships node 12 and @bufferapp/cli needs >= 18.
# The CLI is pinned; `buffer update` from inside a session is not the way.
RUN curl -fsSL https://deb.nodesource.com/setup_22.x | bash - \
 && apt-get install -y --no-install-recommends nodejs \
 && rm -rf /var/lib/apt/lists/* \
 && npm install -g @bufferapp/cli@1.2.0 \
 && npm cache clean --force

# Non-root. /workspace is the per-worker tool root, /mnt/memory is where the
# SDK materialises memory stores if a session ever attaches one (it needs to
# be writable by the worker user; the SDK creates the per-store dirs itself).
RUN useradd --create-home --uid 1000 worker \
 && mkdir -p /workspace /mnt/memory \
 && chown worker:worker /workspace /mnt/memory

COPY --chown=worker:worker worker.py /app/worker.py

USER worker
WORKDIR /workspace
VOLUME ["/workspace"]

# Long-running: polls the environment's work queue and serves sessions in this
# process. SIGTERM → the worker cancels in-flight work, posts error results,
# force-stops the work item, exits (docker-compose's default stop grace is 10 s;
# compose sets 40 s so a memory flush can finish).
CMD ["python3", "/app/worker.py"]
