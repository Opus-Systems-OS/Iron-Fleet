"""Iron-Fleet rig-gpu worker: Anthropic's SDK EnvironmentWorker, always-on.

This is the always-on shape from the self-hosted-sandboxes docs, verbatim in
spirit: poll the environment's work queue, and for each claimed session attach
to its event stream, run `agent.tool_use` calls locally (bash/read/write/edit/
glob/grep from `agent_toolset_20260401`), post `user.tool_result` events back,
heartbeat the work-item lease meanwhile, force-stop the item when the session
goes idle. The agent loop stays on Anthropic's side (CLAUDE.md).

Configuration is environment-only, so the key never lands in an image or the
repo:

  ANTHROPIC_ENVIRONMENT_ID   env_...            required
  ANTHROPIC_ENVIRONMENT_KEY  sk-ant-oat01-...   required; Console-generated
  ANTHROPIC_BASE_URL         optional, default https://api.anthropic.com
  WORKER_WORKDIR             optional, default /workspace
  WORKER_MAX_IDLE_SECONDS    optional, default 60 — how long to keep serving a
                             session after it goes idle with end_turn
  WORKER_MEMORY_SYNC_SECONDS optional, default 15 (min 5); "off" disables
                             memory-store mounting entirely
  WORKER_LOG_LEVEL           optional, default INFO
"""

from __future__ import annotations

import asyncio
import contextlib
import logging
import os
import signal
import sys

from anthropic import AsyncAnthropic
from anthropic.lib.environments import EnvironmentWorker


def _require(name: str) -> str:
    value = os.environ.get(name)
    if not value:
        sys.exit(f"{name} is required (see worker/sdk/README.md)")
    return value


def _memory_sync_interval() -> float | None:
    raw = os.environ.get("WORKER_MEMORY_SYNC_SECONDS", "15").strip().lower()
    if raw in ("off", "none", "0"):
        return None
    return float(raw)


async def main() -> None:
    logging.basicConfig(
        level=os.environ.get("WORKER_LOG_LEVEL", "INFO").upper(),
        format="%(asctime)s %(levelname)s %(name)s: %(message)s",
    )
    log = logging.getLogger("iron-fleet-worker")

    environment_id = _require("ANTHROPIC_ENVIRONMENT_ID")
    environment_key = _require("ANTHROPIC_ENVIRONMENT_KEY")
    workdir = os.environ.get("WORKER_WORKDIR", "/workspace")
    max_idle = float(os.environ.get("WORKER_MAX_IDLE_SECONDS", "60"))
    os.makedirs(workdir, exist_ok=True)

    log.info(
        "starting environment_id=%s workdir=%s max_idle=%ss base_url=%s",
        environment_id,
        workdir,
        max_idle,
        os.environ.get("ANTHROPIC_BASE_URL", "https://api.anthropic.com"),
    )

    # auth_token, not api_key: the worker authenticates as the environment,
    # never as the account. There is no ANTHROPIC_API_KEY on the rig.
    async with AsyncAnthropic(auth_token=environment_key) as client:
        worker = EnvironmentWorker(
            client,
            environment_id=environment_id,
            environment_key=environment_key,
            workdir=workdir,
            max_idle=max_idle,
            memory_sync_interval=_memory_sync_interval(),
        )
        task = asyncio.create_task(worker.run())
        # Cancel the task rather than kill the process: the worker then stops
        # its in-flight tool call, posts an error result, flushes memory, and
        # force-stops the work item so it doesn't sit until the lease lapses.
        loop = asyncio.get_running_loop()
        for signum in (signal.SIGINT, signal.SIGTERM):
            loop.add_signal_handler(signum, task.cancel)
        with contextlib.suppress(asyncio.CancelledError):
            await task
    log.info("stopped")


if __name__ == "__main__":
    asyncio.run(main())
