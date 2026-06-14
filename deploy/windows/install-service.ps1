param(
    [string]$InferraExe = "",
    [string]$InstallRoot = "",
    [string]$ProgramDataRoot = "$env:ProgramData\Inferra",
    [string]$ConfigPath = "",
    [string]$DataDir = "",
    [int]$PreferredPort = 7433,
    [string]$PreferredHost = "127.0.0.1",
    [switch]$AllowFirewall,
    [switch]$AddCliToPath,
    [switch]$KillInferraProcessesBeforeInstall,
    [bool]$RegisterService = $true
)

$ErrorActionPreference = "Stop"

Import-Module -Name (Join-Path $PSScriptRoot "InferraInstall.psm1") -Force
Assert-InferraAdministrator

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

function Read-InferraConfigPort {
    param(
        [string]$Path,
        [string]$InferraExe = ""
    )
    $port = 7433
    if ($InferraExe -and (Test-Path $InferraExe) -and (Test-Path $Path)) {
        try {
            $jsonText = (
                & $InferraExe @("--config", $Path, "--json", "config", "get", "server.port") 2>$null |
                Out-String
            ).Trim()
            if ($LASTEXITCODE -eq 0 -and $jsonText) {
                $obj = $jsonText | ConvertFrom-Json
                if ($null -ne $obj.value) {
                    return [int]$obj.value
                }
            }
        }
        catch {
            # fall through to TOML scan
        }
    }
    if (Test-Path $Path) {
        $match = Select-String -Path $Path -Pattern '^\s*port\s*=\s*(\d+)' | Select-Object -First 1
        if ($match) {
            $port = [int]$match.Matches[0].Groups[1].Value
        }
    }
    return $port
}

function Read-InferraConfigHost {
    param(
        [string]$Path,
        [string]$InferraExe = ""
    )
    $hostValue = "127.0.0.1"
    if ($InferraExe -and (Test-Path $InferraExe) -and (Test-Path $Path)) {
        try {
            $jsonText = (
                & $InferraExe @("--config", $Path, "--json", "config", "get", "server.host") 2>$null |
                Out-String
            ).Trim()
            if ($LASTEXITCODE -eq 0 -and $jsonText) {
                $obj = $jsonText | ConvertFrom-Json
                if ($null -ne $obj.value -and "$($obj.value)".Trim()) {
                    return "$($obj.value)".Trim()
                }
            }
        }
        catch {
            # fall through to TOML scan
        }
    }
    if (Test-Path $Path) {
        $match = Select-String -Path $Path -Pattern '^\s*host\s*=\s*"([^"]+)"' | Select-Object -First 1
        if ($match) {
            $hostValue = $match.Matches[0].Groups[1].Value.Trim()
        }
    }
    return $hostValue
}

function Get-InferraClientHost {
    param([string]$BindHost)
    $normalized = "$BindHost".Trim()
    switch ($normalized) {
        "" { return "127.0.0.1" }
        "0.0.0.0" { return "127.0.0.1" }
        "::" { return "[::1]" }
        "[::]" { return "[::1]" }
        default {
            if ($normalized.Contains(":") -and -not $normalized.StartsWith("[")) {
                return "[$normalized]"
            }
            return $normalized
        }
    }
}

function Resolve-InferraBindIPAddress {
    param([string]$BindHost)
    $normalized = "$BindHost".Trim()
    switch ($normalized) {
        "" { return [System.Net.IPAddress]::Loopback }
        "0.0.0.0" { return [System.Net.IPAddress]::Any }
        "::" { return [System.Net.IPAddress]::IPv6Any }
        "[::]" { return [System.Net.IPAddress]::IPv6Any }
        default {
            $ipAddress = $null
            if ([System.Net.IPAddress]::TryParse($normalized.Trim('[', ']'), [ref]$ipAddress)) {
                return $ipAddress
            }
            $resolved = [System.Net.Dns]::GetHostAddresses($normalized) | Select-Object -First 1
            if ($null -eq $resolved) {
                throw "Could not resolve bind host '$BindHost' to an IP address."
            }
            return $resolved
        }
    }
}

function Test-InferraPortBindable {
    param(
        [string]$BindHost,
        [int]$Port
    )
    $listener = $null
    try {
        $address = Resolve-InferraBindIPAddress -BindHost $BindHost
        $listener = [System.Net.Sockets.TcpListener]::new($address, $Port)
        $listener.Start()
        return $true
    }
    catch {
        return $false
    }
    finally {
        if ($listener) {
            $listener.Stop()
        }
    }
}

function Get-InferraFreeTcpPort {
    param([string]$BindHost)
    $listener = $null
    try {
        $address = Resolve-InferraBindIPAddress -BindHost $BindHost
        $listener = [System.Net.Sockets.TcpListener]::new($address, 0)
        $listener.Start()
        return ([System.Net.IPEndPoint]$listener.LocalEndpoint).Port
    }
    finally {
        if ($listener) {
            $listener.Stop()
        }
    }
}

function Get-ZombieListenerPids {
    param([int]$Port)
    $pids = @()
    $pattern = ":\s*$Port\s+.*LISTENING"
    foreach ($line in (netstat -ano | Select-String $pattern)) {
        $procId = ($line -split '\s+')[-1]
        if ($procId -match '^\d+$') {
            $pids += [int]$procId
        }
    }
    return $pids | Select-Object -Unique
}

function Test-ZombiePortListener {
    param([int]$Port)
    foreach ($procId in (Get-ZombieListenerPids -Port $Port)) {
        if (-not (Get-Process -Id $procId -ErrorAction SilentlyContinue)) {
            return $true
        }
    }
    return $false
}

function Resolve-InferraDashboardPort {
    param(
        [Parameter(Mandatory = $true)][string]$InstalledExe,
        [Parameter(Mandatory = $true)][string]$ConfigPath,
        [Parameter(Mandatory = $true)][string]$BindHost,
        [int]$ConfiguredPort,
        [int]$PreferredPort,
        [string]$PreferredHost
    )

    $clientHost = Get-InferraClientHost -BindHost $BindHost
    $healthUrl = "http://${clientHost}:${ConfiguredPort}/api/health"

    if (Test-ZombiePortListener -Port $ConfiguredPort) {
        Write-Warning "Port $ConfiguredPort has a zombie listener (TCP open but owning process is gone). Reboot Windows to clear ghost ports 7433-7437, or Inferra will keep using alternate ports."
    }

    if ($ConfiguredPort -ne $PreferredPort -and (Test-InferraPortBindable -BindHost $PreferredHost -Port $PreferredPort)) {
        Write-Host "Using preferred port $PreferredPort on $PreferredHost."
        Invoke-InferraCommand $InstalledExe @("--config", $ConfigPath, "config", "set", "server.host", $PreferredHost)
        Invoke-InferraCommand $InstalledExe @("--config", $ConfigPath, "config", "set", "server.port", "$PreferredPort")
        return @{
            BindHost = $PreferredHost
            Port = $PreferredPort
            HealthUrl = "http://$(Get-InferraClientHost -BindHost $PreferredHost):${PreferredPort}/api/health"
        }
    }

    if (Test-InferraPortBindable -BindHost $BindHost -Port $ConfiguredPort) {
        return @{
            BindHost = $BindHost
            Port = $ConfiguredPort
            HealthUrl = $healthUrl
        }
    }

    $existingInferra = Test-InferraHealthEndpoint -Url $healthUrl -TimeoutSec 3
    $replacementPort = Get-InferraFreeTcpPort -BindHost $PreferredHost
    $reason = if ($existingInferra) {
        "an existing Inferra runtime is already responding there"
    }
    else {
        "another listener is occupying the port without answering Inferra health checks"
    }
    Write-Warning "Configured port $ConfiguredPort on $BindHost is unavailable because $reason. Switching Inferra to free port $replacementPort on $PreferredHost."
    Invoke-InferraCommand $InstalledExe @("--config", $ConfigPath, "config", "set", "server.host", $PreferredHost)
    Invoke-InferraCommand $InstalledExe @("--config", $ConfigPath, "config", "set", "server.port", "$replacementPort")
  return @{
        BindHost = $PreferredHost
        Port = $replacementPort
        HealthUrl = "http://$(Get-InferraClientHost -BindHost $PreferredHost):${replacementPort}/api/health"
    }
}

function Test-InferraHealthEndpoint {
    param(
        [Parameter(Mandatory = $true)][string]$Url,
        [int]$TimeoutSec = 5
    )
    try {
        if ($PSVersionTable.PSVersion.Major -ge 7) {
            $resp = Invoke-WebRequest -Uri $Url -UseBasicParsing -TimeoutSec $TimeoutSec -NoProxy
        }
        else {
            $ws = New-Object Microsoft.PowerShell.Commands.WebRequestSession
            $ws.Proxy = $null
            $resp = Invoke-WebRequest -Uri $Url -WebSession $ws -UseBasicParsing -TimeoutSec $TimeoutSec
        }
        return ($resp.StatusCode -eq 200)
    }
    catch {
        return $false
    }
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

$layout = Get-InferraInstallLayout -InstallRoot $InstallRoot -ProgramDataRoot $ProgramDataRoot
$installBinDir = $layout.BinDir
$installRuntimeAssets = $layout.RuntimeAssets
$installedExe = $layout.ExePath
$installedUiDist = $layout.UiDist
$sourceRuntimeAssets = Resolve-InferraRuntimeAssetsSource -SourceExe $InferraExe -ProjectRoot $projectRoot
$sourceUiDist = Join-Path $projectRoot "src\web\ui_dist"
$sourceDefaultsToml = Join-Path $projectRoot "src\config\defaults.toml"
$versionText = ""
try {
    $versionText = (& $InferraExe --version 2>&1 | Out-String).Trim()
} catch {
    # optional
}

$existing = Get-Service -Name "Inferra" -ErrorAction SilentlyContinue
if ($existing -and $RegisterService) {
    Stop-Service Inferra -ErrorAction SilentlyContinue
    Invoke-InferraCommand $InferraExe @("service", "remove")
    Start-Sleep -Seconds 2
}

Install-InferraRuntimePayload `
    -Layout $layout `
    -SourceExe $InferraExe `
    -SourceUiDist $(if (Test-Path $sourceUiDist) { $sourceUiDist } else { (Join-Path $sourceRuntimeAssets "ui_dist") }) `
    -SourceRuntimeAssets $sourceRuntimeAssets `
    -SourceDefaultsToml $(if (Test-Path $sourceDefaultsToml) { $sourceDefaultsToml } else { "" }) `
    -VersionText $versionText

if (-not (Test-Path $ConfigPath)) {
    try {
        Push-Location $installBinDir
        Invoke-InferraCommand $installedExe @("--config", $ConfigPath, "setup", "--yes", "--skip-connection-test", "--data-dir", $DataDir)
    } finally {
        Pop-Location
    }
}

try {
    Push-Location $installBinDir
    Invoke-InferraCommand $installedExe @("--config", $ConfigPath, "init-db")
}
finally {
    Pop-Location
}

$bindHost = Read-InferraConfigHost -Path $ConfigPath -InferraExe $installedExe
$dashPort = Read-InferraConfigPort -Path $ConfigPath -InferraExe $installedExe
$resolved = Resolve-InferraDashboardPort `
    -InstalledExe $installedExe `
    -ConfigPath $ConfigPath `
    -BindHost $bindHost `
    -ConfiguredPort $dashPort `
    -PreferredPort $PreferredPort `
    -PreferredHost $PreferredHost
$bindHost = $resolved.BindHost
$dashPort = $resolved.Port
$healthUrl = $resolved.HealthUrl
$clientHost = Get-InferraClientHost -BindHost $bindHost

$serveLog = $layout.ServeLog
if ($RegisterService) {
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

    Start-Sleep -Seconds 2
    $reachable = $false
    for ($attempt = 0; $attempt -lt 20; $attempt++) {
        if (Test-InferraHealthEndpoint -Url $healthUrl -TimeoutSec 5) {
            $reachable = $true
            break
        }
        if ($attempt -lt 19) {
            Start-Sleep -Seconds 2
        }
    }
    if ($reachable) {
        Write-Host "Runtime health: $healthUrl"
        Write-Host "Dashboard: http://${clientHost}:${dashPort}/"
        Write-Host ""
        Write-Host "Control commands (new terminal after -AddCliToPath):"
        Write-Host "  inferra runtime status"
        Write-Host "  inferra runtime open"
        Write-Host "  inferra runtime restart"
    }
    else {
        Write-Warning "Runtime health not reachable at $healthUrl (service may still be starting). If this step was very slow, check Windows proxy settings for localhost; see $serveLog for service host errors."
        Write-Host "Serve log (Windows service lifecycle + HTTP errors): $serveLog"
        Write-Host "Try: Restart-Service Inferra"
    }
}
else {
    Write-Host "Skipped Windows service registration (RegisterService=`$false)."
}
Write-Host "Serve log: $serveLog"

if ($AllowFirewall) {
    $port = Read-InferraConfigPort -Path $ConfigPath -InferraExe $installedExe
    $ruleName = "Inferra-HTTP-$port"
    $existingRule = Get-NetFirewallRule -DisplayName $ruleName -ErrorAction SilentlyContinue
    if (-not $existingRule) {
        New-NetFirewallRule -DisplayName $ruleName -Direction Inbound -Action Allow -Protocol TCP -LocalPort $port | Out-Null
    }
}

if ($AddCliToPath) {
    Add-InferraBinToMachinePath -BinDir $installBinDir
    Set-InferraMachineConfigEnv -ConfigPath $ConfigPath
    Write-InferraPathReport -BinDir $installBinDir
}

Write-Host "Installed runtime root: $($layout.InstallRoot)"
Write-Host "Installed executable: $installedExe"
Write-Host "Installed UI bundle: $installedUiDist"
Write-Host "Installed defaults reference: $($layout.DefaultsToml)"
if ($RegisterService) {
    Write-Host "Inferra service installed and started (config=$ConfigPath data=$DataDir)."
} else {
    Write-Host "Inferra files installed without service (config=$ConfigPath data=$DataDir)."
}
