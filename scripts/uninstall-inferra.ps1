# Remove Inferra Windows service, install tree, PATH entries, and optionally all program data.
# Run in elevated PowerShell.
#requires -Version 5.1
param(
    [switch]$Full,
    [switch]$KeepData,
    [string]$InstallRoot = '',
    [string]$ProgramDataRoot = "$env:ProgramData\Inferra"
)

$ErrorActionPreference = 'Stop'
$Root = Split-Path -Parent $PSScriptRoot
$installModule = Join-Path $Root 'deploy\windows\InferraInstall.psm1'
$windowsModule = Join-Path $Root 'deploy\windows\InferraWindows.psm1'

Import-Module -Name $installModule -Force
Assert-InferraAdministrator

$layout = Get-InferraInstallLayout -InstallRoot $InstallRoot -ProgramDataRoot $ProgramDataRoot
Write-Host 'Inferra uninstall'
Write-Host "  Install root: $($layout.InstallRoot)"
Write-Host "  Program data: $($layout.ProgramDataRoot)"
Write-Host "  Mode: $(if ($Full -and -not $KeepData) { 'Full (remove service, install tree, PATH, and program data)' } else { 'Standard (keep config/data under ProgramData)' })"

Write-InferraPathReport -BinDir $layout.BinDir -Heading 'PATH status before uninstall'

if (Test-Path $windowsModule) {
    Import-Module -Name $windowsModule -Force
    Stop-InferraWindowsExecutionLocks -ServiceStopTimeoutSec 120
}

$serviceScript = Join-Path $Root 'deploy\windows\uninstall-service.ps1'
& $serviceScript -InstallRoot $layout.InstallRoot -InferraExe $layout.ExePath

Remove-InferraBinFromMachinePath -BinDir $layout.BinDir | Out-Null
Remove-InferraMachineConfigEnv | Out-Null
Remove-InferraFirewallRules
Remove-InferraInstallRoot -InstallRoot $layout.InstallRoot | Out-Null

if ($Full -and -not $KeepData) {
    Remove-InferraProgramDataRoot -ProgramDataRoot $layout.ProgramDataRoot | Out-Null
}
elseif ($Full -and $KeepData) {
    Write-Host "Full uninstall requested but -KeepData preserved $($layout.ProgramDataRoot)"
}
else {
    Write-Host "Kept program data at $($layout.ProgramDataRoot) (use -Full to remove config, data, and logs)."
}

Write-InferraPathReport -BinDir $layout.BinDir -Heading 'PATH status after uninstall'
Write-Host 'Inferra uninstall complete.'
