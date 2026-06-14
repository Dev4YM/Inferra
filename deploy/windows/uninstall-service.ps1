param(
    [string]$InferraExe = "",
    [string]$InstallRoot = ""
)

$ErrorActionPreference = "Stop"

Import-Module -Name (Join-Path $PSScriptRoot "InferraInstall.psm1") -Force

$projectRoot = (Resolve-Path (Join-Path $PSScriptRoot "..\..")).Path
$layout = Get-InferraInstallLayout -InstallRoot $InstallRoot

if (-not $InferraExe) {
    $candidates = @(
        $layout.ExePath,
        (Join-Path $projectRoot "dist\inferra-rust.exe"),
        (Join-Path $projectRoot "dist\inferra.exe")
    )
    foreach ($candidate in $candidates) {
        if (Test-Path $candidate) {
            $InferraExe = $candidate
            break
        }
    }
    if (-not $InferraExe) {
        $inferraCommand = Get-Command inferra -ErrorAction SilentlyContinue
        if ($inferraCommand) {
            $InferraExe = $inferraCommand.Source
        }
    }
}

function Invoke-InferraCommand {
    param(
        [Parameter(Mandatory = $true)][string]$FilePath,
        [Parameter(Mandatory = $true)][string[]]$Arguments
    )

    if (-not (Test-Path $FilePath)) {
        throw "Inferra executable not found: $FilePath"
    }

    & $FilePath @Arguments
    if ($LASTEXITCODE -ne 0) {
        throw "Command failed with exit code ${LASTEXITCODE}: $FilePath $($Arguments -join ' ')"
    }
}

$existing = Get-Service -Name "Inferra" -ErrorAction SilentlyContinue
if ($existing) {
    Stop-Service Inferra -ErrorAction SilentlyContinue
    if ($InferraExe -and (Test-Path $InferraExe)) {
        Invoke-InferraCommand $InferraExe @("service", "remove")
    }
    else {
        & sc.exe delete Inferra | Out-Null
    }
    Write-Host "Inferra service removed."
}
else {
    Write-Host "Inferra service is not installed."
}

Remove-InferraFirewallRules
