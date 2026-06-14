# Install Inferra on Windows: build from this repo, stage under Program Files, register service.
# Run in elevated PowerShell.
#requires -Version 5.1
param(
    [switch]$Full,
    [switch]$SkipBuild,
    [switch]$SkipStopDev,
    [switch]$NoService,
    [switch]$NoPath,
    [switch]$AllowFirewall,
    [string]$InstallRoot = '',
    [string]$ProgramDataRoot = "$env:ProgramData\Inferra",
    [int]$PreferredPort = 7433,
    [string]$PreferredHost = '127.0.0.1'
)

$ErrorActionPreference = 'Stop'
$Root = Split-Path -Parent $PSScriptRoot
$installModule = Join-Path $Root 'deploy\windows\InferraInstall.psm1'
$windowsModule = Join-Path $Root 'deploy\windows\InferraWindows.psm1'

Import-Module -Name $installModule -Force
Assert-InferraAdministrator

$layout = Get-InferraInstallLayout -InstallRoot $InstallRoot -ProgramDataRoot $ProgramDataRoot
Write-Host 'Inferra install'
Write-Host "  Install root: $($layout.InstallRoot)"
Write-Host "  Program data: $($layout.ProgramDataRoot)"
Write-Host "  Mode: $(if ($Full) { 'Full (clean build + complete install)' } else { 'Standard (incremental build when needed)' })"

if (-not $SkipStopDev) {
    & (Join-Path $Root 'scripts\stop-inferra-dev.ps1')
}

if (-not $SkipBuild) {
    $buildArgs = @{}
    if ($Full) { $buildArgs.Full = $true }
    & (Join-Path $Root 'scripts\build-all.ps1') @buildArgs
}

$artifacts = Resolve-InferraProjectArtifacts -RepoRoot $Root
$versionText = ''
try {
    $versionText = (& $artifacts.Exe --version 2>&1 | Out-String).Trim()
}
catch {
    Write-Warning "Could not read inferra --version from $($artifacts.Exe)"
}

if ($Full -and (Test-Path $windowsModule)) {
    Import-Module -Name $windowsModule -Force
    Stop-InferraWindowsExecutionLocks -ServiceStopTimeoutSec 120
}

$serviceScript = Join-Path $Root 'deploy\windows\install-service.ps1'
$serviceParams = @{
    InferraExe = $artifacts.Exe
    InstallRoot = $layout.InstallRoot
    ProgramDataRoot = $layout.ProgramDataRoot
    ConfigPath = $layout.ConfigPath
    DataDir = $layout.DataDir
    PreferredPort = $PreferredPort
    PreferredHost = $PreferredHost
    KillInferraProcessesBeforeInstall = [bool]$Full
}

if ($AllowFirewall) { $serviceParams.AllowFirewall = $true }
if (-not $NoPath) { $serviceParams.AddCliToPath = $true }
if ($NoService) { $serviceParams.RegisterService = $false }

& $serviceScript @serviceParams

Write-InferraPathReport -BinDir $layout.BinDir -Heading 'Post-install PATH check'

Write-Host ''
Write-Host 'Installed layout:'
Write-Host "  CLI/core: $($layout.ExePath)"
Write-Host "  Web UI:   $($layout.UiDist)"
Write-Host "  Defaults: $($layout.DefaultsToml)"
Write-Host "  Config:   $($layout.ConfigPath)"
Write-Host "  Data:     $($layout.DataDir)"
Write-Host "  Logs:     $($layout.LogsDir)"
if ($versionText) {
    Write-Host "  Version:  $versionText"
}
