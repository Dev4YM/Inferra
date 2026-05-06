$ErrorActionPreference = "Stop"
$Root = Split-Path -Parent $MyInvocation.MyCommand.Path
Set-Location (Join-Path $Root "..\src\web\frontend")
npm ci
npm run build
Write-Host "Built web UI to src/web/ui_dist"
