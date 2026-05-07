#requires -Version 5.1
<#
.SYNOPSIS
    Compatibility shim for the deprecated PyInstaller build venv helper.
#>
param(
    [string]$Python = 'python',
    [string]$VenvDir = '',
    [switch]$SkipPipUpgrade
)

$ErrorActionPreference = 'Stop'

$legacyScript = Join-Path $PSScriptRoot '..\..\deprecated\windows-pyinstaller\prepare-build-venv.ps1'
Write-Warning "deploy/windows/prepare-build-venv.ps1 is deprecated. Prefer the Rust-native build, or run the archived script at $legacyScript."

& $legacyScript -Python $Python -VenvDir $VenvDir -SkipPipUpgrade:$SkipPipUpgrade
exit $LASTEXITCODE
