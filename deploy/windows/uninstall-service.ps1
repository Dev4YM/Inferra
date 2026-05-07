param(
    [string]$InferraExe = "",
    [string]$InstallRoot = ""
)

$ErrorActionPreference = "Stop"

$projectRoot = (Resolve-Path (Join-Path $PSScriptRoot "..\..")).Path
if (-not $InstallRoot) {
    $InstallRoot = Join-Path ([Environment]::GetFolderPath("ProgramFiles")) "Inferra"
}
if (-not $InferraExe) {
    $preferredExe = Join-Path $InstallRoot "bin\inferra.exe"
    $fallbackNativeExe = Join-Path $projectRoot "dist\inferra-rust.exe"
    $stableExe = Join-Path $projectRoot "dist\inferra.exe"
    if (Test-Path $preferredExe) {
        $InferraExe = $preferredExe
    } elseif (Test-Path $fallbackNativeExe) {
        $InferraExe = $fallbackNativeExe
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
    throw "Could not find a Rust Inferra executable. Build dist\inferra-rust.exe or install inferra on PATH before running uninstall-service.ps1."
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

$existing = Get-Service -Name "Inferra" -ErrorAction SilentlyContinue
if ($existing) {
    Stop-Service Inferra -ErrorAction SilentlyContinue
    Invoke-InferraCommand $InferraExe @("service", "remove")
    Write-Host "Inferra service removed."
} else {
    Write-Host "Inferra service is not installed."
}

Get-NetFirewallRule -ErrorAction SilentlyContinue | Where-Object { $_.DisplayName -like "Inferra-HTTP-*" } | ForEach-Object {
    Remove-NetFirewallRule -InputObject $_ -ErrorAction SilentlyContinue
}
