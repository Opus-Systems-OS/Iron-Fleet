# The rig: local inference (Phase 6)

Ollama on the Windows rig's RTX 5070, reachable from the droplet's
control plane over Tailscale and from nowhere else. The control plane's
`/inference/*` routes (`control-plane/README.md`) proxy to it; the rig
being off is a `503`, never a fallback to Claude.

This is written for a Claude Code session on the rig, same shape as
`deploy/droplet/README.md`: a file table, the first-run steps in order,
checks, and what to record back.

| File | What |
|---|---|
| `models.txt` | The committed model list, one tag per line. The only sanctioned way to put a model on the rig. |
| `setup.ps1` | Idempotent first run / re-run: user env vars, Ollama restart, missing pulls, `tailscale serve`, prints the tailnet IP. |
| `check.ps1` | Read-only health dump: `ollama list` / `ps`, `nvidia-smi`, `/api/tags`, env, `tailscale status` + `serve status`. |

## Shape

```
droplet (control-plane container) ──tailnet──> rig 100.x.y.z:11434 (tailscale serve)
                                                       └──> 127.0.0.1:11434 (ollama)
```

- Ollama binds **`127.0.0.1` only** (`OLLAMA_HOST`). No firewall rule, no
  port forward, no LAN exposure.
- `tailscale serve --tcp=11434` forwards the rig's tailnet address to that
  loopback port. Only tailnet members reach it; the tailnet's members are
  the rig, the droplet, and whatever you log in.
- `OLLAMA_MAX_LOADED_MODELS=1`, `OLLAMA_KEEP_ALIVE=5m`: one model resident,
  unloaded after five idle minutes, so a `gpu-compute` session gets its
  VRAM back. Revisit only on a real session OOM.

## Prerequisites

1. **Ollama ≥ 0.6** — Blackwell (the 5070) needs the CUDA 12.8 build.
   `ollama --version`. After the first prompt, `ollama ps` must show
   `100% GPU`; `CPU` there means the CUDA runtime didn't load and nothing
   below is worth doing until it does.
2. **Tailscale** installed and logged in (tray icon → Log in; GitHub
   identity is fine). `"C:\Program Files\Tailscale\tailscale.exe" status`
   shows the rig with a `100.x` address. The installer doesn't add the CLI
   to an already-open shell's PATH; the scripts look in the install dir.
3. `nvidia-smi` on PATH (comes with the driver).

## First run

From the repo checkout, a normal (not elevated) PowerShell:

```powershell
powershell -ExecutionPolicy Bypass -File deploy\rig\setup.ps1
```

It sets the three `OLLAMA_*` user env vars (restarting the tray app if any
changed), pulls the tags in `models.txt` that aren't in `ollama list`
(~5.3 GB the first time), runs `tailscale serve --bg --tcp=11434
tcp://127.0.0.1:11434`, and prints the tailnet IPv4 as an
`INFERENCE_URL=` line. Re-running is safe and quick.

Then warm the model once and confirm it lands on the GPU:

```powershell
ollama run qwen3:8b "one word: hello"
ollama ps            # expect qwen3:8b ... 100% GPU
```

## Checks

```powershell
powershell -ExecutionPolicy Bypass -File deploy\rig\check.ps1
```

From another tailnet member (the Mac, once it's logged in, or the droplet):

```sh
curl -s http://<rig tailnet IP>:11434/api/tags
```

The two model tags back means the serve forward works. From the droplet
the check is inside the compose network — see "Rig link" in
`deploy/droplet/README.md`.

## Record these back

Into `docs/centralization-plan.md` "Resume here" (Phase 6 block):

- The rig's tailnet IPv4 and hostname (`tailscale status` first line).
- `ollama --version` and the `ollama ps` line showing `100% GPU`.
- `/api/tags` output (model names + sizes).
- `nvidia-smi` memory with `qwen3:8b` loaded.

The droplet side then sets `INFERENCE_URL=http://<rig tailnet IP>:11434`
in its `.env` and runs `deploy.sh`.

## Ollama at login

The Windows installer puts `Ollama.lnk` in the user's Startup folder, so
the tray app (and with it `ollama serve`) comes up when the rig auto-logs
in for the worker. If a reboot ever leaves `/api/version` unanswered,
that shortcut is gone: recreate it, or add a Task Scheduler "At log on"
task running `"%LOCALAPPDATA%\Programs\Ollama\ollama app.exe"`. Don't
reach for NSSM — a service would run without the user env vars above.

## Taking it down

`tailscale serve --tcp=11434 off` stops the tailnet forward; quitting the
tray app stops Ollama. Either way the control plane answers `503
rig_offline` within ~5 s and nothing else on the droplet notices.
