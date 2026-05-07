#requires -Version 5.1
<#
.SYNOPSIS
    Compatibility shim for the deprecated PyInstaller build.
#>
param(
    [string]$Python = 'python',
    [switch]$SkipReleaseLocks,
    [switch]$CleanPyInstallerWork,
    [switch]$NoSmokeTest,
    [ValidateRange(1, 600)][int]$LockReleaseTimeoutSec = 120,
    [ValidateRange(1, 500)][int]$PublishCopyAttempts = 48
)

$ErrorActionPreference = 'Stop'

$legacyScript = Join-Path $PSScriptRoot '..\..\deprecated\windows-pyinstaller\build-exe.ps1'
Write-Warning "deploy/windows/build-exe.ps1 is deprecated. Use deploy/windows/build-rust-exe.ps1 for the native runtime, or run the archived script at $legacyScript."

& $legacyScript `
    -Python $Python `
    -SkipReleaseLocks:$SkipReleaseLocks `
    -CleanPyInstallerWork:$CleanPyInstallerWork `
    -NoSmokeTest:$NoSmokeTest `
    -LockReleaseTimeoutSec $LockReleaseTimeoutSec `
    -PublishCopyAttempts $PublishCopyAttempts
exit $LASTEXITCODE
