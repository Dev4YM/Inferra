# Start Inferra API (dev config) and the Vite console together.
$ErrorActionPreference = "Stop"
$Root = Split-Path -Parent $PSScriptRoot
$DevConfig = Join-Path $Root "inferra.dev.toml"
$DefaultConfig = Join-Path $Root "inferra.toml"
$Inferra = Join-Path $Root "src\target\debug\inferra.exe"

if (-not (Test-Path $DevConfig)) {
  if (-not (Test-Path $DefaultConfig)) {
    throw "Missing inferra.toml; cannot create inferra.dev.toml"
  }
  Copy-Item $DefaultConfig $DevConfig
  Write-Host "Created inferra.dev.toml from inferra.toml (gitignored — adjust server.port / cors for local dev)."
}

if (-not (Test-Path $Inferra)) {
  Write-Host "Building inferra debug binary..."
  Push-Location (Join-Path $Root "src")
  cargo build -p inferra-cli
  Pop-Location
}

Write-Host "Checking for listeners on 7433 / 7434..."
foreach ($port in @(7433, 7434)) {
  $line = netstat -ano | Select-String "127.0.0.1:$port\s+.*LISTENING" | Select-Object -First 1
  if ($line) {
    $pid = ($line -split '\s+')[-1]
    Write-Host "  port $port -> PID $pid"
  }
}

Write-Host "Starting inferra serve with $DevConfig"
$api = Start-Process -FilePath $Inferra -ArgumentList @("serve", "--config", $DevConfig) -WorkingDirectory $Root -PassThru -WindowStyle Hidden
Start-Sleep -Seconds 2

Write-Host "Starting Vite (INFERRA_API_URL from .env.development)"
Push-Location (Join-Path $Root "src\web\frontend")
try {
  npm run dev
} finally {
  Pop-Location
  if ($api -and -not $api.HasExited) {
    Write-Host "Stopping inferra PID $($api.Id)"
    Stop-Process -Id $api.Id -Force -ErrorAction SilentlyContinue
  }
}
