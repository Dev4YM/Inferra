#requires -Version 5.1
<#
.SYNOPSIS
    Create a dedicated .venv-inferra-build for reproducible PyInstaller builds (isolates site-packages).

.DESCRIPTION
    Heavy unrelated packages in your global Python (or PYTHONPATH) can be pulled into Analysis.
    This venv installs inferra[windows,build-windows] (PyInstaller is pinned there).

    Pip 24+ refuses to upgrade itself when invoked as Scripts\pip.exe. All installs use
    python.exe -m pip (see pip issue #3164 / deprecation notices).

.PARAMETER SkipPipUpgrade
    Skip "pip wheel" bootstrap and install the project with the venv's bundled pip only.
    Use if corporate policy blocks pip self-upgrade (editable install may still work).

.EXAMPLE
    .\deploy\windows\prepare-build-venv.ps1
.EXAMPLE
    .\deploy\windows\prepare-build-venv.ps1 -Python py -SkipPipUpgrade
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
    # Never call Scripts\pip.exe to upgrade pip — pip exits with "To modify pip, run python.exe -m pip ..."
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

Write-Host "[Inferra] Done. Build with:"
Write-Host "  .\deploy\windows\build-exe.ps1 -Python `"$py`""
