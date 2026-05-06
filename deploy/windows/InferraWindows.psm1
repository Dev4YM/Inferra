# Inferra Windows helpers: PyInstaller staging/promotion and execution lock release.
#Requires -Version 5.1

Set-StrictMode -Version 3.0
$ErrorActionPreference = 'Stop'

$script:InferraDeployWindowsDir = $PSScriptRoot

function Get-InferraRepositoryRoot {
    return (Resolve-Path (Join-Path $script:InferraDeployWindowsDir '..\..')).Path
}

function Get-InferraPackageVersion {
    param(
        [Parameter(Mandatory)][string]$RepoRoot,
        [Parameter(Mandatory)][string]$Python
    )
    $pp = Join-Path $RepoRoot 'pyproject.toml'
    if (-not (Test-Path -LiteralPath $pp)) {
        throw "Missing pyproject.toml: $pp"
    }
    # Do not use python -c with Path/"pyproject.toml" — without quotes Python parses
    # r/pyproject.toml as r / (pyproject.toml) and raises NameError: pyproject.
    $reader = Join-Path $script:InferraDeployWindowsDir 'read_pyproject_version.py'
    if (-not (Test-Path -LiteralPath $reader)) {
        throw "Missing version helper script: $reader"
    }
    $raw = & $Python $reader $RepoRoot
    if ($LASTEXITCODE -ne 0) {
        throw "Could not read version from pyproject.toml via ${Python} ${reader}: $raw"
    }
    return "$raw".Trim()
}

function Assert-PythonRunnable {
    param([Parameter(Mandatory)][string]$Python)
    $null = & $Python -c "import sys; sys.exit(0)" 2>&1
    if ($LASTEXITCODE -ne 0) {
        throw "Python is not runnable: $Python (exit $LASTEXITCODE)"
    }
}

function Assert-PyInstallerAvailable {
    param([Parameter(Mandatory)][string]$Python)
    $null = & $Python -c "import PyInstaller" 2>&1
    if ($LASTEXITCODE -ne 0) {
        throw "PyInstaller is not installed for this interpreter. Install with: $Python -m pip install pyinstaller"
    }
}

function Stop-InferraWindowsServiceIfInstalled {
    param([int]$TimeoutSec = 120)

    $svc = Get-Service -Name 'Inferra' -ErrorAction SilentlyContinue
    if ($null -eq $svc) {
        return
    }
    if ($svc.Status -eq 'Stopped') {
        return
    }

    Write-Host "[Inferra] Stopping Windows service 'Inferra' (status was $($svc.Status))..."
    try {
        Stop-Service -Name 'Inferra' -Force -ErrorAction Stop
    }
    catch {
        Write-Warning "[Inferra] Stop-Service failed (${_}); attempting sc.exe stop Inferra"
        $null = & sc.exe stop Inferra 2>&1
    }

    $deadline = (Get-Date).AddSeconds($TimeoutSec)
    while ((Get-Date) -lt $deadline) {
        $svc = Get-Service -Name 'Inferra' -ErrorAction Stop
        if ($svc.Status -eq 'Stopped') {
            return
        }
        Start-Sleep -Seconds 1
    }

    throw "Inferra service did not reach Stopped within ${TimeoutSec}s (last status: $($svc.Status)). Inspect dependents (Get-Service), Event Log, then retry."
}

function Stop-InferraInferraExeProcesses {
    param([int]$MaxAttempts = 16)

    for ($attempt = 1; $attempt -le $MaxAttempts; $attempt++) {
        $procs = @(Get-CimInstance Win32_Process -Filter "Name = 'inferra.exe'" -ErrorAction SilentlyContinue)
        if ($procs.Count -eq 0) {
            return
        }

        Write-Host "[Inferra] Stopping inferra.exe instance(s) (attempt $attempt/$MaxAttempts): $($procs.ProcessId -join ', ')"
        foreach ($p in $procs) {
            Stop-Process -Id $p.ProcessId -Force -ErrorAction SilentlyContinue
        }
        Start-Sleep -Milliseconds 600
    }

    $left = @(Get-CimInstance Win32_Process -Filter "Name = 'inferra.exe'" -ErrorAction SilentlyContinue)
    if ($left.Count -gt 0) {
        $ids = $left.ProcessId -join ', '
        throw "inferra.exe is still running (PIDs: $ids). Close those processes, stop the Inferra service, or reboot, then retry the build."
    }
}

function Stop-InferraWindowsExecutionLocks {
    <#
    .SYNOPSIS
        Stop the Inferra service (if installed) and terminate inferra.exe processes.
        Used before promoting dist\inferra.exe or when replacing a running service binary.
    #>
    param([int]$ServiceStopTimeoutSec = 120)

    Stop-InferraWindowsServiceIfInstalled -TimeoutSec $ServiceStopTimeoutSec
    Stop-InferraInferraExeProcesses -MaxAttempts 16
    Stop-InferraInferraExeProcesses -MaxAttempts 6
}

function Copy-InferraFileWithRetry {
    param(
        [Parameter(Mandatory)][string]$Source,
        [Parameter(Mandatory)][string]$Destination,
        [int]$Attempts = 48,
        [int]$DelayMs = 500
    )

    if (-not (Test-Path -LiteralPath $Source)) {
        throw "Source missing: $Source"
    }

    $destDir = Split-Path -Parent $Destination
    if (-not (Test-Path -LiteralPath $destDir)) {
        New-Item -ItemType Directory -Path $destDir -Force | Out-Null
    }

    $last = $null
    for ($i = 1; $i -le $Attempts; $i++) {
        try {
            Copy-Item -LiteralPath $Source -Destination $Destination -Force -ErrorAction Stop
            return
        }
        catch {
            $last = $_
            Start-Sleep -Milliseconds $DelayMs
        }
    }

    throw "Failed to copy '$Source' -> '$Destination' after $Attempts attempts: $last"
}

function Test-InferraBuiltExe {
    param(
        [Parameter(Mandatory)][string]$ExePath,
        [Parameter(Mandatory)][string]$ExpectedVersion
    )

    $out = & $ExePath --version 2>&1
    if ($LASTEXITCODE -ne 0) {
        throw "Smoke test failed: '$ExePath --version' exited $LASTEXITCODE : $out"
    }

    $text = "$out"
    if ($text -notmatch [regex]::Escape($ExpectedVersion)) {
        Write-Warning "[Inferra] '--version' output did not contain '$ExpectedVersion': $text"
    }
}

function Invoke-InferraWindowsExeBuild {
    <#
    .SYNOPSIS
        Full Windows one-file build: optional lock release, isolated PyInstaller output directory,
        promote to dist\inferra.exe + dist\inferra-<version>.exe, smoke test.

    .NOTES
        PyInstaller never writes directly to dist\inferra.exe — it writes under dist\_inferra_exe_stage\,
        avoiding PermissionError when the previous binary is loaded by the SCM or child process.
    #>
    [CmdletBinding()]
    param(
        [string]$Python = 'python',
        [switch]$SkipReleaseLocks,
        [switch]$CleanPyInstallerWork,
        [switch]$NoSmokeTest,
        [ValidateRange(1, 600)][int]$LockReleaseTimeoutSec = 120,
        [ValidateRange(1, 500)][int]$PublishCopyAttempts = 48
    )

    $repo = Get-InferraRepositoryRoot
    $packageVersion = Get-InferraPackageVersion -RepoRoot $repo -Python $Python

    $stageDir = Join-Path $repo 'dist\_inferra_exe_stage'
    $stageExe = Join-Path $stageDir 'inferra.exe'
    $workPath = Join-Path $repo 'build\inferra_exe_work'
    $finalExe = Join-Path $repo 'dist\inferra.exe'
    $versionedExe = Join-Path $repo "dist\inferra-$packageVersion.exe"

    Assert-PythonRunnable -Python $Python
    Assert-PyInstallerAvailable -Python $Python

    if (-not $SkipReleaseLocks) {
        Stop-InferraWindowsExecutionLocks -ServiceStopTimeoutSec $LockReleaseTimeoutSec
    }

    if ($CleanPyInstallerWork -and (Test-Path -LiteralPath $workPath)) {
        Write-Host "[Inferra] Removing PyInstaller work directory: $workPath"
        Remove-Item -LiteralPath $workPath -Recurse -Force -ErrorAction Stop
    }

    New-Item -ItemType Directory -Path $stageDir -Force | Out-Null
    Get-ChildItem -LiteralPath $stageDir -Force -ErrorAction SilentlyContinue |
        Remove-Item -Recurse -Force -ErrorAction SilentlyContinue

    $specRel = 'deploy\windows\inferra.spec'
    $savedPyPath = $env:PYTHONPATH
    Remove-Item Env:\PYTHONPATH -ErrorAction SilentlyContinue

    try {
        Push-Location $repo
        Write-Host "[Inferra] PyInstaller: distpath=$stageDir workpath=$workPath"
        & $Python -m PyInstaller --noconfirm --distpath $stageDir --workpath $workPath $specRel
        if ($LASTEXITCODE -ne 0) {
            throw "PyInstaller failed with exit code $LASTEXITCODE"
        }
    }
    finally {
        Pop-Location
        if ($null -ne $savedPyPath) {
            Set-Item -Path Env:PYTHONPATH -Value $savedPyPath
        }
        else {
            Remove-Item Env:\PYTHONPATH -ErrorAction SilentlyContinue
        }
    }

    if (-not (Test-Path -LiteralPath $stageExe)) {
        throw "PyInstaller did not produce expected artifact: $stageExe"
    }

    $size = (Get-Item -LiteralPath $stageExe).Length
    if ($size -lt 50KB) {
        throw "Staged exe is implausibly small ($size bytes): $stageExe"
    }

    if (-not $NoSmokeTest) {
        Test-InferraBuiltExe -ExePath $stageExe -ExpectedVersion $packageVersion
    }

    try {
        Copy-InferraFileWithRetry -Source $stageExe -Destination $versionedExe -Attempts $PublishCopyAttempts
        if (-not $SkipReleaseLocks) {
            Stop-InferraWindowsExecutionLocks -ServiceStopTimeoutSec $LockReleaseTimeoutSec
        }
        Copy-InferraFileWithRetry -Source $stageExe -Destination $finalExe -Attempts $PublishCopyAttempts
    }
    catch {
        Write-Error $_
        Write-Host "[Inferra] Promotion failed; staged binary is usable here: $stageExe"
        Write-Host "[Inferra] Run Stop-InferraWindowsExecutionLocks (import InferraWindows.psm1) or reboot, then re-run this script."
        return 2
    }

    Write-Host "[Inferra] Primary artifact: $finalExe"
    Write-Host "[Inferra] Versioned artifact: $versionedExe"
    Write-Host "[Inferra] Staged copy retained at: $stageExe"
    return 0
}

Export-ModuleMember -Function @(
    'Get-InferraRepositoryRoot',
    'Get-InferraPackageVersion',
    'Stop-InferraWindowsExecutionLocks',
    'Invoke-InferraWindowsExeBuild'
)
