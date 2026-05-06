#requires -Version 5.1
<#
.SYNOPSIS
    Production Windows one-file build for Inferra (PyInstaller staging + promote + smoke test).

.DESCRIPTION
    Loads deploy/windows/InferraWindows.psm1 and runs Invoke-InferraWindowsExeBuild.
    PyInstaller writes to dist\_inferra_exe_stage\ only; dist\inferra.exe is updated via copy with retries.

.PARAMETER SkipReleaseLocks
    Skip stopping the Inferra service and inferra.exe processes. Use on clean CI agents.

.EXAMPLE
    .\deploy\windows\build-exe.ps1

.EXAMPLE
    .\deploy\windows\build-exe.ps1 -Python .\.venv-inferra-build\Scripts\python.exe

.EXAMPLE
    .\deploy\windows\build-exe.ps1 -SkipReleaseLocks -CleanPyInstallerWork
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

$modulePath = Join-Path $PSScriptRoot 'InferraWindows.psm1'
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
