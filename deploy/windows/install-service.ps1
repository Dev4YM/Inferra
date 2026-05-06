param(
    [string]$Python = "python",
    [string]$InferraExe = "",
    [string]$ProgramDataRoot = "$env:ProgramData\Inferra",
    [string]$ConfigPath = "",
    [string]$DataDir = "",
    [switch]$SkipPipInstall,
    [switch]$AllowFirewall,
    [switch]$AddCliToPath,
    [switch]$KillInferraProcessesBeforeInstall
)

$ErrorActionPreference = "Stop"

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

if (-not $ConfigPath) { $ConfigPath = Join-Path $ProgramDataRoot "inferra.toml" }
if (-not $DataDir) { $DataDir = Join-Path $ProgramDataRoot "data" }

New-Item -ItemType Directory -Force -Path $ProgramDataRoot | Out-Null
New-Item -ItemType Directory -Force -Path $DataDir | Out-Null

$inheritanceOff = "/inheritance:r"
$grantSystem = 'SYSTEM:(OI)(CI)F'
$grantAdmins = 'Administrators:(OI)(CI)F'
icacls $ProgramDataRoot $inheritanceOff /grant:r $grantSystem /grant:r $grantAdmins | Out-Null
icacls $DataDir $inheritanceOff /grant:r $grantSystem /grant:r $grantAdmins | Out-Null

$projectRoot = (Resolve-Path (Join-Path $PSScriptRoot "..\..")).Path

if (-not (Test-Path $ConfigPath)) {
    try {
        Push-Location $projectRoot
        $setupArgs = @(
            "-m", "cli",
            "--config", $ConfigPath,
            "setup",
            "--yes",
            "--skip-connection-test",
            "--data-dir", $DataDir
        )
        if ($InferraExe) {
            Invoke-InferraCommand $InferraExe @("--config", $ConfigPath, "setup", "--yes", "--skip-connection-test", "--data-dir", $DataDir)
        } else {
            Invoke-InferraCommand $Python $setupArgs
        }
    } finally {
        Pop-Location
    }
}

try {
    Push-Location $projectRoot
    if ($InferraExe) {
        Invoke-InferraCommand $InferraExe @("--config", $ConfigPath, "init-db")
    }
    else {
        Invoke-InferraCommand $Python @("-m", "cli", "--config", $ConfigPath, "init-db")
    }
}
finally {
    Pop-Location
}

$existing = Get-Service -Name "Inferra" -ErrorAction SilentlyContinue
if ($existing) {
    Stop-Service Inferra -ErrorAction SilentlyContinue
    if ($InferraExe) {
        Invoke-InferraCommand $InferraExe @("remove")
    } else {
        Invoke-InferraCommand $Python @("-m", "windows_service", "remove")
    }
    Start-Sleep -Seconds 2
}

if (-not $SkipPipInstall -and -not $InferraExe) {
    try {
        Push-Location $projectRoot
        Invoke-InferraCommand $Python @("-m", "pip", "install", "-e", ".[windows]")
    } finally {
        Pop-Location
    }
}

$installArgs = @(
    "--startup", "auto",
    "install",
    "--config", $ConfigPath,
    "--data-dir", $DataDir
)

if ($InferraExe) {
    Invoke-InferraCommand $InferraExe $installArgs
} else {
    Invoke-InferraCommand $Python (@("-m", "windows_service") + $installArgs)
}

$registered = Get-Service -Name "Inferra" -ErrorAction Stop
Start-Service -Name $registered.Name -ErrorAction Stop

$dashPort = Read-InferraConfigPort -Path $ConfigPath
Start-Sleep -Seconds 2
$reachable = $false
for ($attempt = 0; $attempt -lt 12; $attempt++) {
    try {
        $resp = Invoke-WebRequest -Uri "http://127.0.0.1:${dashPort}/" -UseBasicParsing -TimeoutSec 25
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
    Write-Host "Dashboard: http://127.0.0.1:${dashPort}/"
}
else {
    Write-Warning "Dashboard not reachable at http://127.0.0.1:${dashPort}/ (service may still be starting)."
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
    if ($InferraExe) {
        $binDir = Join-Path $ProgramDataRoot "bin"
        New-Item -ItemType Directory -Force -Path $binDir | Out-Null
        Copy-Item -Path $InferraExe -Destination (Join-Path $binDir "inferra.exe") -Force
        Add-InferraBinToMachinePath -BinDir $binDir
    }
    else {
        $scriptsDir = (& $Python -c "import sysconfig; print(sysconfig.get_path('scripts'))").Trim()
        if (-not $scriptsDir) {
            throw "Could not resolve Python scripts directory for PATH."
        }
        Add-InferraBinToMachinePath -BinDir $scriptsDir
    }
}

Write-Host "Inferra service installed and started (config=$ConfigPath data=$DataDir)."
