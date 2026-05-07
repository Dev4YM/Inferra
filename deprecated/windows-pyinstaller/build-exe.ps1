#requires -Version 5.1
<#
.SYNOPSIS
    Deprecated Python-first one-file build for Inferra (PyInstaller).

.DESCRIPTION
    This path is archived under deprecated/windows-pyinstaller because Inferra
    now ships the Rust runtime shell via deploy/windows/build-rust-exe.ps1.
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

$modulePath = Join-Path $PSScriptRoot '..\..\deploy\windows\InferraWindows.psm1'
Import-Module -Name $modulePath -Force

$invokeParams = @{
    Python                = $Python
    SkipReleaseLocks      = $SkipReleaseLocks
    CleanPyInstallerWork  = $CleanPyInstallerWork
    NoSmokeTest           = $NoSmokeTest
    LockReleaseTimeoutSec = $LockReleaseTimeoutSec
    PublishCopyAttempts   = $PublishCopyAttempts
}

try {
    $exitCode = Invoke-InferraWindowsExeBuild @invokeParams
    exit [int]$exitCode
}
catch {
    Write-Error $_
    exit 1
}
