param(
    [string]$Python = "python",
    [string]$ProjectDir = (Resolve-Path (Join-Path $PSScriptRoot "..\..")).Path,
    [string]$ConfigPath = "$env:ProgramData\Inferra\inferra.toml",
    [switch]$SkipInstall
)

$ErrorActionPreference = "Stop"

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

$programData = Split-Path -Parent $ConfigPath
New-Item -ItemType Directory -Force -Path $programData | Out-Null

if (-not (Test-Path $ConfigPath)) {
    try {
        Push-Location $ProjectDir
        Invoke-InferraCommand $Python @(
            "-m",
            "cli",
            "--config",
            $ConfigPath,
            "setup",
            "--yes",
            "--skip-connection-test",
            "--data-dir",
            "$env:ProgramData\Inferra\data"
        )
    } finally {
        Pop-Location
    }
}

$existing = Get-Service -Name "Inferra" -ErrorAction SilentlyContinue
if ($existing) {
    Stop-Service Inferra -ErrorAction SilentlyContinue
    Invoke-InferraCommand $Python @("-m", "windows_service", "remove") | Out-Null
    Start-Sleep -Seconds 2
}

if (-not $SkipInstall) {
    try {
        Push-Location $ProjectDir
        Invoke-InferraCommand $Python @("-m", "pip", "install", "-e", ".[windows]")
    } finally {
        Pop-Location
    }
}

$env:INFERRA_CONFIG = $ConfigPath
Invoke-InferraCommand $Python @("-m", "windows_service", "--startup", "auto", "install")
$registered = Get-Service -Name "Inferra" -ErrorAction Stop
Start-Service -Name $registered.Name -ErrorAction Stop
Write-Host "Inferra service installed and started."
