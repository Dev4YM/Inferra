param(
    [string]$Python = "python",
    [string]$ProjectDir = (Resolve-Path (Join-Path $PSScriptRoot "..\..")).Path,
    [string]$ConfigPath = "$env:ProgramData\Inferra\inferra.toml",
    [switch]$SkipInstall
)

$ErrorActionPreference = "Stop"
$programData = Split-Path -Parent $ConfigPath
New-Item -ItemType Directory -Force -Path $programData | Out-Null

if (-not (Test-Path $ConfigPath)) {
    Push-Location $ProjectDir
    & $Python -m cli --config $ConfigPath setup --yes --skip-connection-test --data-dir "$env:ProgramData\Inferra\data"
    Pop-Location
}

$existing = Get-Service -Name "Inferra" -ErrorAction SilentlyContinue
if ($existing) {
    Stop-Service Inferra -ErrorAction SilentlyContinue
    & $Python -m windows_service remove | Out-Null
    Start-Sleep -Seconds 2
}

if (-not $SkipInstall) {
    Push-Location $ProjectDir
    & $Python -m pip install -e ".[windows]"
    Pop-Location
}

$env:INFERRA_CONFIG = $ConfigPath
& $Python -m windows_service install --startup auto
Start-Service Inferra
Write-Host "Inferra service installed and started."
