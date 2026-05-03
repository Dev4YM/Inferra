$Python = "python"
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

$existing = Get-Service -Name "Inferra" -ErrorAction SilentlyContinue
if ($existing) {
    Stop-Service Inferra -ErrorAction SilentlyContinue
    Invoke-InferraCommand $Python @("-m", "windows_service", "remove") | Out-Null
    Write-Host "Inferra service removed."
} else {
    Write-Host "Inferra service is not installed."
}
