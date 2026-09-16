# Rig-side setup for Phase 6 (local inference). Idempotent; run as the
# logged-in user from a normal (not elevated) PowerShell:
#   powershell -ExecutionPolicy Bypass -File deploy\rig\setup.ps1
#
# What it does, in order:
#   1. Pins Ollama to 127.0.0.1 and sets the GPU-sharing knobs as *user*
#      environment variables, then restarts the tray app so they take.
#   2. Pulls every tag in models.txt that `ollama list` doesn't have.
#   3. Publishes 127.0.0.1:11434 to the tailnet with `tailscale serve`
#      (TCP forward, persisted with --bg). Nothing is opened at home.
#   4. Prints the tailnet IPv4 and the serve status - paste both back.
#
# Nothing here needs admin: user env vars, user-scope Ollama, and tailscale
# serve all work from the interactive user's session.

$ErrorActionPreference = 'Stop'
$here = Split-Path -Parent $MyInvocation.MyCommand.Path

# Native exes (ollama pull writes its progress to stderr) under 'Stop' in
# Windows PowerShell 5.1 turn stderr into a terminating error once output is
# redirected; run them with 'Continue' and judge by the exit code.
function Invoke-Native {
  param([string]$Exe, [string[]]$Arguments)
  $ea = $ErrorActionPreference
  $ErrorActionPreference = 'Continue'
  try { & $Exe @Arguments } finally { $ErrorActionPreference = $ea }
  if ($LASTEXITCODE -ne 0) { throw "$([IO.Path]::GetFileName($Exe)) $($Arguments -join ' ') failed (exit $LASTEXITCODE)" }
}

# --- tools --------------------------------------------------------------
$ollama = (Get-Command ollama -ErrorAction SilentlyContinue).Source
if (-not $ollama) { throw "ollama not on PATH - install from https://ollama.com/download/windows" }
$ollamaApp = Join-Path (Split-Path -Parent $ollama) 'ollama app.exe'

# The Tailscale installer doesn't add its CLI to PATH for a shell that was
# already open; look in the install dir before giving up.
$tailscale = (Get-Command tailscale -ErrorAction SilentlyContinue).Source
if (-not $tailscale) { $tailscale = Join-Path $env:ProgramFiles 'Tailscale\tailscale.exe' }
if (-not (Test-Path $tailscale)) { throw "tailscale.exe not found - install from https://tailscale.com/download/windows and log in first" }

Write-Host "ollama:    $(& $ollama --version)"
Write-Host "tailscale: $((& $tailscale version)[0])"

# --- 1. environment -----------------------------------------------------
# User scope: survives reboots, applies to the tray app started at logon,
# never touches the machine-wide environment.
$wanted = [ordered]@{
  OLLAMA_HOST              = '127.0.0.1:11434'   # loopback only; tailscale serve is the only way in
  OLLAMA_KEEP_ALIVE        = '5m'                # unload after idle so a gpu-compute session gets the VRAM back
  OLLAMA_MAX_LOADED_MODELS = '1'                 # chat OR embed resident, never both
}
$changed = $false
foreach ($k in $wanted.Keys) {
  $cur = [Environment]::GetEnvironmentVariable($k, 'User')
  if ($cur -ne $wanted[$k]) {
    [Environment]::SetEnvironmentVariable($k, $wanted[$k], 'User')
    Write-Host "env: $k=$($wanted[$k]) (was '$cur')"
    $changed = $true
  } else {
    Write-Host "env: $k=$cur (unchanged)"
  }
  Set-Item -Path "Env:$k" -Value $wanted[$k]   # for this process, so the restart below inherits it
}

if ($changed) {
  # Ollama on Windows is the tray app ("ollama app.exe") supervising
  # "ollama.exe serve"; stop both, start the tray app, which relaunches the
  # server with the new environment.
  Write-Host "ollama: restarting so the environment takes"
  Get-Process -Name 'ollama app', 'ollama' -ErrorAction SilentlyContinue | Stop-Process -Force
  Start-Sleep -Seconds 2
  Start-Process -FilePath $ollamaApp
  $deadline = (Get-Date).AddSeconds(30)
  do {
    Start-Sleep -Seconds 1
    try { $v = (Invoke-RestMethod 'http://127.0.0.1:11434/api/version' -TimeoutSec 2).version } catch { $v = $null }
  } until ($v -or (Get-Date) -gt $deadline)
  if (-not $v) { throw "ollama did not come back on 127.0.0.1:11434 within 30 s" }
  Write-Host "ollama: back, version $v"
} else {
  try { $v = (Invoke-RestMethod 'http://127.0.0.1:11434/api/version' -TimeoutSec 2).version }
  catch { throw "ollama is not answering on 127.0.0.1:11434 - start it (Start Menu > Ollama) and re-run" }
  Write-Host "ollama: running, version $v"
}

# --- 2. models ----------------------------------------------------------
$have = @()
try { $have = (Invoke-RestMethod 'http://127.0.0.1:11434/api/tags' -TimeoutSec 5).models | ForEach-Object { $_.name } } catch {}
$tags = Get-Content (Join-Path $here 'models.txt') |
  ForEach-Object { ($_ -split '#')[0].Trim() } |
  Where-Object { $_ }
foreach ($tag in $tags) {
  # `ollama list` shows "qwen3:8b"; a bare "name" in models.txt means ":latest".
  $full = if ($tag -match ':') { $tag } else { "$tag`:latest" }
  if ($have -contains $full) {
    Write-Host "model: $tag (present)"
  } else {
    Write-Host "model: pulling $tag"
    Invoke-Native $ollama @('pull', $tag)
  }
}

# --- 3. tailnet ---------------------------------------------------------
$ip = (Invoke-Native $tailscale @('ip', '-4') | Select-Object -First 1)
if (-not $ip) { throw "tailscale has no IPv4 - is it logged in? (tray icon > Log in)" }

# Idempotent: re-running `serve --bg` with the same mapping is a no-op.
Invoke-Native $tailscale @('serve', '--bg', '--tcp=11434', 'tcp://127.0.0.1:11434') | Out-Null

# --- 4. report ----------------------------------------------------------
Write-Host ""
Write-Host "=== paste back to the Mac side ==="
Write-Host "tailnet IPv4:   $ip"
Write-Host "INFERENCE_URL=http://$ip`:11434"
Write-Host ""
Invoke-Native $tailscale @('serve', 'status')
Write-Host ""
Write-Host "models on the rig:"
(Invoke-RestMethod 'http://127.0.0.1:11434/api/tags').models | ForEach-Object { "  $($_.name)  $([math]::Round($_.size / 1GB, 2)) GB" }
