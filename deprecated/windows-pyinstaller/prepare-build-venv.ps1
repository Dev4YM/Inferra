#requires -Version 5.1
<#
.SYNOPSIS
    Deprecated helper for the archived PyInstaller build venv.

.DESCRIPTION
    Retained only so the old Python-first Windows packaging flow stays
    reproducible while the Rust-native release path takes over.
#>
param(
    [string]$Python = 'python',
    [string]$VenvDir = '',
    [switch]$SkipPipUpgrade
)

$ErrorActionPreference = 'Stop'

$repo = (Resolve-Path (Join-Path $PSScriptRoot '..\..')).Path
if (-not $VenvDir) {
    $VenvDir = Join-Path $repo '.venv-inferra-build'
}

Write-Host "[Inferra] Creating build venv at: $VenvDir"

if (Test-Path -LiteralPath $VenvDir) {
    Write-Host "[Inferra] Removing existing venv..."
    Remove-Item -LiteralPath $VenvDir -Recurse -Force -ErrorAction Stop
}

& $Python -m venv $VenvDir
if ($LASTEXITCODE -ne 0) {
    throw "python -m venv failed with exit $LASTEXITCODE"
}

$py = Join-Path $VenvDir 'Scripts\python.exe'
if (-not (Test-Path -LiteralPath $py)) {
    throw "Venv Python missing at $py"
}

if (-not $SkipPipUpgrade) {
    Write-Host '[Inferra] python -m pip install --upgrade pip wheel'
    & $py -m pip install --upgrade pip wheel
    if ($LASTEXITCODE -ne 0) {
        Write-Warning "[Inferra] pip+wheel upgrade failed (exit $LASTEXITCODE); retrying pip only..."
        & $py -m pip install --upgrade pip
        if ($LASTEXITCODE -ne 0) {
            throw @"
pip self-upgrade failed (exit $LASTEXITCODE).

Fix manually, then re-run this script:
  & `"$py`" -m pip install --upgrade pip wheel

Or retry with -SkipPipUpgrade if your environment blocks pip upgrades.
"@
        }
    }
}

Write-Host '[Inferra] python -m pip install -e ".[windows,build-windows]"'
& $py -m pip install -e ($repo + '[windows,build-windows]')
if ($LASTEXITCODE -ne 0) {
    throw "Editable install failed (exit $LASTEXITCODE). Command: `"$py`" -m pip install -e ($repo + '[windows,build-windows]')"
}

Write-Host "[Inferra] Done. Legacy build with:"
Write-Host "  .\deprecated\windows-pyinstaller\build-exe.ps1 -Python `"$py`""
