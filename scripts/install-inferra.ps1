# Build and install Inferra as a Windows service (API + dashboard). Run in elevated PowerShell.
#requires -Version 5.1
param(
    [switch]$SkipBuild,
    [switch]$SkipStopDev,
    [int]$PreferredPort = 7433
)

$ErrorActionPreference = "Stop"
$Root = Split-Path -Parent $PSScriptRoot

if (-not $SkipStopDev) {
    & (Join-Path $Root "scripts\stop-inferra-dev.ps1")
}

if (-not $SkipBuild) {
    Push-Location (Join-Path $Root "src\web\frontend")
    try {
        npm run build
    }
    finally {
        Pop-Location
    }
    & (Join-Path $Root "deploy\windows\build-rust-exe.ps1") -CopyUiBundle
}

$exe = Resolve-Path (Join-Path $Root "dist\inferra-rust.exe")
& (Join-Path $Root "deploy\windows\install-service.ps1") `
    -InferraExe $exe `
    -KillInferraProcessesBeforeInstall `
    -AddCliToPath `
    -PreferredPort $PreferredPort `
    -PreferredHost "127.0.0.1"
