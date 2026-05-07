param(
    [string]$InferraExe = "",
    [string]$InstallRoot = "",
    [string]$ProgramDataRoot = "$env:ProgramData\Inferra",
    [string]$ConfigPath = "",
    [string]$DataDir = "",
    [switch]$AllowFirewall,
    [switch]$AddCliToPath,
    [switch]$KillInferraProcessesBeforeInstall
)

$ErrorActionPreference = "Stop"

$projectRoot = (Resolve-Path (Join-Path $PSScriptRoot "..\..")).Path
if (-not $InstallRoot) {
    $InstallRoot = Join-Path ([Environment]::GetFolderPath("ProgramFiles")) "Inferra"
}
if (-not $InferraExe) {
    $preferredExe = Join-Path $projectRoot "dist\inferra-rust.exe"
    $stableExe = Join-Path $projectRoot "dist\inferra.exe"
    if (Test-Path $preferredExe) {
        $InferraExe = $preferredExe
    } elseif (Test-Path $stableExe) {
        $InferraExe = $stableExe
    } else {
        $inferraCommand = Get-Command inferra -ErrorAction SilentlyContinue
        if ($inferraCommand) {
            $InferraExe = $inferraCommand.Source
        }
    }
}
if (-not $InferraExe) {
    throw "Could not find a Rust Inferra executable. Build dist\inferra-rust.exe or install inferra on PATH before running install-service.ps1."
}
$InferraExe = (Resolve-Path $InferraExe).Path

if ($KillInferraProcessesBeforeInstall) {
    $wm = Join-Path $PSScriptRoot "InferraWindows.psm1"
    Import-Module -Name $wm -Force
    Stop-InferraWindowsExecutionLocks -ServiceStopTimeoutSec 120
}

function Add-InferraBinToMachinePath {
    param([Parameter(Mandatory = $true)][string]$BinDir)
    $BinDir = [System.IO.Path]::GetFullPath($BinDir.TrimEnd('\', '/'))
    $machinePath = [Environment]::GetEnvironmentVariable("Path", "Machine")
    $parts = @()
    if ($machinePath) {
        $parts = $machinePath.Split(';') | Where-Object { $_ }
    }
    foreach ($p in $parts) {
        $norm = [System.IO.Path]::GetFullPath($p.TrimEnd('\', '/'))
        if ($norm -ieq $BinDir) {
            Write-Host "PATH already contains: $BinDir"
            return
        }
    }
    $newPath = if ($machinePath) { "$machinePath;$BinDir" } else { $BinDir }
    [Environment]::SetEnvironmentVariable("Path", $newPath, "Machine")
    Write-Host "Added to Machine PATH: $BinDir (open a new terminal for inferra)."
}

function Read-InferraConfigPort {
    param([string]$Path)
    $port = 7433
    if (Test-Path $Path) {
        $match = Select-String -Path $Path -Pattern '^\s*port\s*=\s*(\d+)' | Select-Object -First 1
        if ($match) {
            $port = [int]$match.Matches[0].Groups[1].Value
        }
    }
    return $port
}

function Invoke-InferraCommand {
    param(
        [Parameter(Mandatory = $true)][string]$FilePath,
        [Parameter(Mandatory = $true)][string[]]$Arguments
    )

    & $FilePath @Arguments
    if ($LASTEXITCODE -ne 0) {
        throw "Command failed with exit code ${LASTEXITCODE}: $FilePath $($Arguments -join ' ')"
    }
}

function Resolve-InferraRuntimeAssetsSource {
    param(
        [Parameter(Mandatory = $true)][string]$SourceExe,
        [Parameter(Mandatory = $true)][string]$ProjectRoot
    )

    $exeDir = Split-Path $SourceExe -Parent
    $candidates = @(
        (Join-Path $exeDir "runtime-assets"),
        (Join-Path (Split-Path $exeDir -Parent) "runtime-assets"),
        (Join-Path $ProjectRoot "dist\runtime-assets")
    ) | Where-Object { $_ -and (Test-Path $_) }

    foreach ($candidate in $candidates) {
        if (Test-Path (Join-Path $candidate "ui_dist")) {
            return (Resolve-Path $candidate).Path
        }
    }

    throw "Could not find runtime-assets for $SourceExe. Build with .\deploy\windows\build-rust-exe.ps1 -CopyUiBundle or provide an installed inferra.exe that already has runtime-assets."
}

function Sync-InferraRuntimeAssets {
    param(
        [Parameter(Mandatory = $true)][string]$SourceRoot,
        [Parameter(Mandatory = $true)][string]$DestinationRoot
    )

    New-Item -ItemType Directory -Force -Path $DestinationRoot | Out-Null
    Remove-Item (Join-Path $DestinationRoot "*") -Recurse -Force -ErrorAction SilentlyContinue
    Copy-Item (Join-Path $SourceRoot "*") $DestinationRoot -Recurse -Force
}

if (-not $ConfigPath) { $ConfigPath = Join-Path $ProgramDataRoot "inferra.toml" }
if (-not $DataDir) { $DataDir = Join-Path $ProgramDataRoot "data" }

$installBinDir = Join-Path $InstallRoot "bin"
$installRuntimeAssets = Join-Path $InstallRoot "runtime-assets"
$installedExe = Join-Path $installBinDir "inferra.exe"
$installedUiDist = Join-Path $installRuntimeAssets "ui_dist"
$sourceRuntimeAssets = Resolve-InferraRuntimeAssetsSource -SourceExe $InferraExe -ProjectRoot $projectRoot

New-Item -ItemType Directory -Force -Path $ProgramDataRoot | Out-Null
New-Item -ItemType Directory -Force -Path $DataDir | Out-Null
New-Item -ItemType Directory -Force -Path $InstallRoot | Out-Null
New-Item -ItemType Directory -Force -Path $installBinDir | Out-Null

$inheritanceOff = "/inheritance:r"
$grantSystem = 'SYSTEM:(OI)(CI)F'
$grantAdmins = 'Administrators:(OI)(CI)F'
icacls $ProgramDataRoot $inheritanceOff /grant:r $grantSystem /grant:r $grantAdmins | Out-Null
icacls $DataDir $inheritanceOff /grant:r $grantSystem /grant:r $grantAdmins | Out-Null

$existing = Get-Service -Name "Inferra" -ErrorAction SilentlyContinue
if ($existing) {
    Stop-Service Inferra -ErrorAction SilentlyContinue
    Invoke-InferraCommand $InferraExe @("service", "remove")
    Start-Sleep -Seconds 2
}

Copy-Item -Path $InferraExe -Destination $installedExe -Force
Sync-InferraRuntimeAssets -SourceRoot $sourceRuntimeAssets -DestinationRoot $installRuntimeAssets

if (-not (Test-Path $ConfigPath)) {
    try {
        Push-Location $projectRoot
        Invoke-InferraCommand $installedExe @("--config", $ConfigPath, "setup", "--yes", "--skip-connection-test", "--data-dir", $DataDir)
    } finally {
        Pop-Location
    }
}

try {
    Push-Location $projectRoot
    Invoke-InferraCommand $installedExe @("--config", $ConfigPath, "init-db")
}
finally {
    Pop-Location
}

$installArgs = @(
    "--config", $ConfigPath,
    "--ui-dist", $installedUiDist,
    "service",
    "install",
    "--startup", "auto"
)

Invoke-InferraCommand $installedExe $installArgs

$registered = Get-Service -Name "Inferra" -ErrorAction Stop
Start-Service -Name $registered.Name -ErrorAction Stop

$dashPort = Read-InferraConfigPort -Path $ConfigPath
Start-Sleep -Seconds 2
$reachable = $false
for ($attempt = 0; $attempt -lt 12; $attempt++) {
    try {
        $resp = Invoke-WebRequest -Uri "http://127.0.0.1:${dashPort}/api/health" -UseBasicParsing -TimeoutSec 25
        if ($resp.StatusCode -eq 200) {
            $reachable = $true
            break
        }
    }
    catch {
        if ($attempt -lt 11) {
            Start-Sleep -Seconds 3
        }
    }
}
$serveLog = Join-Path $ProgramDataRoot "logs\serve.log"
if ($reachable) {
    Write-Host "Runtime health: http://127.0.0.1:${dashPort}/api/health"
    Write-Host "Dashboard: http://127.0.0.1:${dashPort}/"
}
else {
    Write-Warning "Runtime health not reachable at http://127.0.0.1:${dashPort}/api/health (service may still be starting)."
    Write-Host "Serve log (stderr/stdout from inferra serve): $serveLog"
    Write-Host "Try: Restart-Service Inferra"
}
Write-Host "Serve log: $serveLog"

if ($AllowFirewall) {
    $port = Read-InferraConfigPort -Path $ConfigPath
    $ruleName = "Inferra-HTTP-$port"
    $existingRule = Get-NetFirewallRule -DisplayName $ruleName -ErrorAction SilentlyContinue
    if (-not $existingRule) {
        New-NetFirewallRule -DisplayName $ruleName -Direction Inbound -Action Allow -Protocol TCP -LocalPort $port | Out-Null
    }
}

if ($AddCliToPath) {
    Add-InferraBinToMachinePath -BinDir $installBinDir
}

Write-Host "Installed runtime root: $InstallRoot"
Write-Host "Installed executable: $installedExe"
Write-Host "Installed UI bundle: $installedUiDist"
Write-Host "Inferra service installed and started (config=$ConfigPath data=$DataDir)."
