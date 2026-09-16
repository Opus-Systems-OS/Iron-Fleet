# Rig health for Phase 6. Read-only; paste the output back.
#   powershell -ExecutionPolicy Bypass -File deploy\rig\check.ps1

$ErrorActionPreference = 'Continue'
$tailscale = (Get-Command tailscale -ErrorAction SilentlyContinue).Source
if (-not $tailscale) { $tailscale = Join-Path $env:ProgramFiles 'Tailscale\tailscale.exe' }

Write-Host "--- ollama list"
ollama list
Write-Host "--- ollama ps   (empty = nothing loaded; after a prompt expect '100% GPU')"
ollama ps
Write-Host "--- nvidia-smi"
nvidia-smi --query-gpu=name,memory.used,memory.total,utilization.gpu --format=csv
Write-Host "--- /api/tags on 127.0.0.1"
try { (Invoke-RestMethod 'http://127.0.0.1:11434/api/tags' -TimeoutSec 5).models | Select-Object name, size | Format-Table -AutoSize }
catch { Write-Host "ollama not answering: $_" }
Write-Host "--- env (user scope)"
foreach ($k in 'OLLAMA_HOST', 'OLLAMA_KEEP_ALIVE', 'OLLAMA_MAX_LOADED_MODELS') {
  "  $k=$([Environment]::GetEnvironmentVariable($k, 'User'))"
}
Write-Host "--- tailscale"
& $tailscale status
& $tailscale serve status
