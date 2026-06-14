# Inferra Windows install layout, PATH management, and file staging helpers.
#Requires -Version 5.1

Set-StrictMode -Version 3.0
$ErrorActionPreference = 'Stop'

$script:InferraDeployWindowsDir = $PSScriptRoot

function Get-InferraRepositoryRoot {
    return (Resolve-Path (Join-Path $script:InferraDeployWindowsDir '..\..')).Path
}

function Get-InferraInstallLayout {
    param(
        [string]$InstallRoot = '',
        [string]$ProgramDataRoot = "$env:ProgramData\Inferra"
    )

    if (-not $InstallRoot) {
        $InstallRoot = Join-Path ([Environment]::GetFolderPath('ProgramFiles')) 'Inferra'
    }

    [ordered]@{
        InstallRoot = $InstallRoot
        ProgramDataRoot = $ProgramDataRoot
        BinDir = Join-Path $InstallRoot 'bin'
        ExePath = Join-Path $InstallRoot 'bin\inferra.exe'
        RuntimeAssets = Join-Path $InstallRoot 'runtime-assets'
        UiDist = Join-Path $InstallRoot 'runtime-assets\ui_dist'
        DefaultsToml = Join-Path $InstallRoot 'runtime-assets\defaults.toml'
        ShareDir = Join-Path $InstallRoot 'share'
        ConfigPath = Join-Path $ProgramDataRoot 'inferra.toml'
        DataDir = Join-Path $ProgramDataRoot 'data'
        LogsDir = Join-Path $ProgramDataRoot 'logs'
        ServeLog = Join-Path $ProgramDataRoot 'logs\serve.log'
    }
}

function Test-InferraAdministrator {
    $identity = [Security.Principal.WindowsIdentity]::GetCurrent()
    $principal = New-Object Security.Principal.WindowsPrincipal($identity)
    return $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
}

function Assert-InferraAdministrator {
    if (-not (Test-InferraAdministrator)) {
        throw 'Inferra install/uninstall requires an elevated PowerShell session (Run as Administrator).'
    }
}

function Get-InferraPathSegments {
    param(
        [ValidateSet('Machine', 'User', 'Process')]
        [string]$Scope = 'Machine'
    )

    $raw = [Environment]::GetEnvironmentVariable('Path', $Scope)
    if (-not $raw) {
        return @()
    }
    return @($raw.Split(';') | Where-Object { $_ })
}

function Test-InferraPathContains {
    param(
        [Parameter(Mandatory = $true)][string]$Directory,
        [ValidateSet('Machine', 'User', 'Process')]
        [string]$Scope = 'Machine'
    )

    $target = [System.IO.Path]::GetFullPath($Directory.TrimEnd('\', '/'))
    foreach ($segment in (Get-InferraPathSegments -Scope $Scope)) {
        $normalized = [System.IO.Path]::GetFullPath($segment.TrimEnd('\', '/'))
        if ($normalized -ieq $target) {
            return $true
        }
    }
    return $false
}

function Get-InferraPathReport {
    param(
        [Parameter(Mandatory = $true)][string]$BinDir
    )

    $machine = Test-InferraPathContains -Directory $BinDir -Scope Machine
    $user = Test-InferraPathContains -Directory $BinDir -Scope User
    $command = Get-Command inferra -ErrorAction SilentlyContinue
    $resolved = if ($command) { $command.Source } else { $null }

    [pscustomobject]@{
        BinDir = $BinDir
        OnMachinePath = $machine
        OnUserPath = $user
        CommandSource = $resolved
        CommandOnPath = [bool]$resolved
    }
}

function Write-InferraPathReport {
    param(
        [Parameter(Mandatory = $true)][string]$BinDir,
        [string]$Heading = 'PATH status'
    )

    $report = Get-InferraPathReport -BinDir $BinDir
    Write-Host ""
    Write-Host $Heading
    Write-Host "  Install bin: $($report.BinDir)"
    Write-Host "  Machine PATH contains bin: $($report.OnMachinePath)"
    Write-Host "  User PATH contains bin: $($report.OnUserPath)"
    if ($report.CommandOnPath) {
        Write-Host "  Current shell resolves inferra -> $($report.CommandSource)"
        if ($report.CommandSource -and ($report.CommandSource -ine (Join-Path $BinDir 'inferra.exe'))) {
            Write-Warning "inferra on PATH points somewhere other than the installed binary. Open a new terminal after install or fix PATH order."
        }
    }
    else {
        Write-Host "  Current shell does not resolve inferra yet (open a new terminal after Machine PATH updates)."
    }
}

function Add-InferraBinToMachinePath {
    param([Parameter(Mandatory = $true)][string]$BinDir)

    $BinDir = [System.IO.Path]::GetFullPath($BinDir.TrimEnd('\', '/'))
    if (Test-InferraPathContains -Directory $BinDir -Scope Machine) {
        Write-Host "Machine PATH already contains: $BinDir"
        return
    }

    $machinePath = [Environment]::GetEnvironmentVariable('Path', 'Machine')
    $newPath = if ($machinePath) { "$machinePath;$BinDir" } else { $BinDir }
    [Environment]::SetEnvironmentVariable('Path', $newPath, 'Machine')
    Write-Host "Added to Machine PATH: $BinDir"
}

function Remove-InferraBinFromMachinePath {
    param([Parameter(Mandatory = $true)][string]$BinDir)

    $BinDir = [System.IO.Path]::GetFullPath($BinDir.TrimEnd('\', '/'))
    $machinePath = [Environment]::GetEnvironmentVariable('Path', 'Machine')
    if (-not $machinePath) {
        return $false
    }

    $parts = @($machinePath.Split(';') | Where-Object { $_ })
    $kept = @()
    $removed = $false
    foreach ($segment in $parts) {
        $normalized = [System.IO.Path]::GetFullPath($segment.TrimEnd('\', '/'))
        if ($normalized -ieq $BinDir) {
            $removed = $true
            continue
        }
        $kept += $segment
    }

    if ($removed) {
        [Environment]::SetEnvironmentVariable('Path', ($kept -join ';'), 'Machine')
        Write-Host "Removed from Machine PATH: $BinDir"
    }
    return $removed
}

function Set-InferraMachineConfigEnv {
    param([Parameter(Mandatory = $true)][string]$ConfigPath)

    [Environment]::SetEnvironmentVariable('INFERRA_CONFIG', $ConfigPath, 'Machine')
    Write-Host "Set machine INFERRA_CONFIG=$ConfigPath"
}

function Remove-InferraMachineConfigEnv {
    $existing = [Environment]::GetEnvironmentVariable('INFERRA_CONFIG', 'Machine')
    if ($existing) {
        [Environment]::SetEnvironmentVariable('INFERRA_CONFIG', $null, 'Machine')
        Write-Host "Removed machine INFERRA_CONFIG (was $existing)"
        return $true
    }
    return $false
}

function Resolve-InferraProjectArtifacts {
    param(
        [Parameter(Mandatory = $true)][string]$RepoRoot
    )

    $distExe = Join-Path $RepoRoot 'dist\inferra-rust.exe'
    $stableExe = Join-Path $RepoRoot 'dist\inferra.exe'
    $uiDist = Join-Path $RepoRoot 'src\web\ui_dist'
    $runtimeAssets = Join-Path $RepoRoot 'dist\runtime-assets'
    $defaultsToml = Join-Path $RepoRoot 'src\config\defaults.toml'

    $exe = if (Test-Path $distExe) { $distExe } elseif (Test-Path $stableExe) { $stableExe } else { $null }
    if (-not $exe) {
        throw "No Inferra executable in dist\. Run scripts\build-all.ps1 or deploy\windows\build-rust-exe.ps1 -CopyUiBundle first."
    }
    if (-not (Test-Path $uiDist)) {
        throw "UI bundle missing at $uiDist. Run scripts\build-all.ps1 or npm run build in src\web\frontend."
    }

    return [pscustomobject]@{
        Exe = (Resolve-Path $exe).Path
        UiDist = (Resolve-Path $uiDist).Path
        RuntimeAssets = if (Test-Path $runtimeAssets) { (Resolve-Path $runtimeAssets).Path } else { $null }
        DefaultsToml = if (Test-Path $defaultsToml) { (Resolve-Path $defaultsToml).Path } else { $null }
    }
}

function Install-InferraRuntimePayload {
    param(
        [Parameter(Mandatory = $true)]$Layout,
        [Parameter(Mandatory = $true)][string]$SourceExe,
        [Parameter(Mandatory = $true)][string]$SourceUiDist,
        [string]$SourceRuntimeAssets = '',
        [string]$SourceDefaultsToml = '',
        [string]$VersionText = ''
    )

    New-Item -ItemType Directory -Force -Path $Layout.InstallRoot | Out-Null
    New-Item -ItemType Directory -Force -Path $Layout.BinDir | Out-Null
    New-Item -ItemType Directory -Force -Path $Layout.RuntimeAssets | Out-Null
    New-Item -ItemType Directory -Force -Path $Layout.ShareDir | Out-Null
    New-Item -ItemType Directory -Force -Path $Layout.ProgramDataRoot | Out-Null
    New-Item -ItemType Directory -Force -Path $Layout.DataDir | Out-Null
    New-Item -ItemType Directory -Force -Path $Layout.LogsDir | Out-Null

    Copy-Item -Path $SourceExe -Destination $Layout.ExePath -Force

    if ($SourceRuntimeAssets -and (Test-Path (Join-Path $SourceRuntimeAssets 'ui_dist'))) {
        Remove-Item (Join-Path $Layout.RuntimeAssets '*') -Recurse -Force -ErrorAction SilentlyContinue
        Copy-Item (Join-Path $SourceRuntimeAssets '*') $Layout.RuntimeAssets -Recurse -Force
    }
    else {
        $uiTarget = $Layout.UiDist
        New-Item -ItemType Directory -Force -Path $uiTarget | Out-Null
        Remove-Item (Join-Path $uiTarget '*') -Recurse -Force -ErrorAction SilentlyContinue
        Copy-Item (Join-Path $SourceUiDist '*') $uiTarget -Recurse -Force
    }

    if ($SourceDefaultsToml) {
        Copy-Item -Path $SourceDefaultsToml -Destination $Layout.DefaultsToml -Force
    }

    if ($VersionText) {
        Set-Content -Path (Join-Path $Layout.ShareDir 'version.txt') -Value $VersionText.Trim() -Encoding UTF8
    }

    $inheritanceOff = '/inheritance:r'
    $grantSystem = 'SYSTEM:(OI)(CI)F'
    $grantAdmins = 'Administrators:(OI)(CI)F'
    icacls $Layout.ProgramDataRoot $inheritanceOff /grant:r $grantSystem /grant:r $grantAdmins | Out-Null
    icacls $Layout.DataDir $inheritanceOff /grant:r $grantSystem /grant:r $grantAdmins | Out-Null
}

function Remove-InferraInstallRoot {
    param(
        [Parameter(Mandatory = $true)][string]$InstallRoot
    )

    if (Test-Path $InstallRoot) {
        Remove-Item -LiteralPath $InstallRoot -Recurse -Force
        Write-Host "Removed install root: $InstallRoot"
        return $true
    }
    Write-Host "Install root not present: $InstallRoot"
    return $false
}

function Remove-InferraProgramDataRoot {
    param(
        [Parameter(Mandatory = $true)][string]$ProgramDataRoot
    )

    if (Test-Path $ProgramDataRoot) {
        Remove-Item -LiteralPath $ProgramDataRoot -Recurse -Force
        Write-Host "Removed program data root: $ProgramDataRoot"
        return $true
    }
    Write-Host "Program data root not present: $ProgramDataRoot"
    return $false
}

function Remove-InferraFirewallRules {
    Get-NetFirewallRule -ErrorAction SilentlyContinue |
        Where-Object { $_.DisplayName -like 'Inferra-HTTP-*' } |
        ForEach-Object {
            Remove-NetFirewallRule -InputObject $_ -ErrorAction SilentlyContinue
        }
}

Export-ModuleMember -Function @(
    'Get-InferraRepositoryRoot',
    'Get-InferraInstallLayout',
    'Test-InferraAdministrator',
    'Assert-InferraAdministrator',
    'Get-InferraPathSegments',
    'Test-InferraPathContains',
    'Get-InferraPathReport',
    'Write-InferraPathReport',
    'Add-InferraBinToMachinePath',
    'Remove-InferraBinFromMachinePath',
    'Set-InferraMachineConfigEnv',
    'Remove-InferraMachineConfigEnv',
    'Resolve-InferraProjectArtifacts',
    'Install-InferraRuntimePayload',
    'Remove-InferraInstallRoot',
    'Remove-InferraProgramDataRoot',
    'Remove-InferraFirewallRules'
)
