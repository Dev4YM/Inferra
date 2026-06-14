# Build Inferra web UI and Rust CLI from the repository, staging dist/runtime-assets.
#requires -Version 5.1
param(
    [switch]$Full,
    [switch]$SkipWeb,
    [switch]$SkipRust,
    [switch]$WebCi,
    [string]$Cargo = 'cargo'
)

$ErrorActionPreference = 'Stop'
$Root = Split-Path -Parent $PSScriptRoot

Write-Host "Inferra build-all (repo: $Root)"

if (-not $SkipWeb) {
    $frontend = Join-Path $Root 'src\web\frontend'
    Push-Location $frontend
    try {
        if ($Full -or $WebCi) {
            Write-Host 'Installing frontend dependencies (npm ci)...'
            npm ci
        }
        Write-Host 'Building web UI...'
        npm run build
    }
    finally {
        Pop-Location
    }
}

if (-not $SkipRust) {
    $buildScript = Join-Path $Root 'deploy\windows\build-rust-exe.ps1'
    if (-not (Test-Path $buildScript)) {
        throw "Missing build script: $buildScript"
    }
    & $buildScript -Cargo $Cargo -CopyUiBundle
    if ($LASTEXITCODE -and $LASTEXITCODE -ne 0) {
        throw "Rust build failed with exit code $LASTEXITCODE"
    }
}

$artifacts = Join-Path $Root 'dist\runtime-assets\ui_dist'
if (-not (Test-Path $artifacts)) {
    throw "Build finished but UI bundle is missing at $artifacts"
}

$exe = Join-Path $Root 'dist\inferra-rust.exe'
if (-not (Test-Path $exe)) {
    $exe = Join-Path $Root 'dist\inferra.exe'
}
if (-not (Test-Path $exe)) {
    throw 'Build finished but inferra executable is missing under dist\'
}

Write-Host ""
Write-Host 'Build complete:'
Write-Host "  CLI: $exe"
Write-Host "  Web: $artifacts"
