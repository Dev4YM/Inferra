param(
    [string]$Python = "python",
    [string]$InferraExe = ""
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

$existing = Get-Service -Name "Inferra" -ErrorAction SilentlyContinue
if ($existing) {
    Stop-Service Inferra -ErrorAction SilentlyContinue
    if ($InferraExe) {
        Invoke-InferraCommand $InferraExe @("remove")
    } else {
        Invoke-InferraCommand $Python @("-m", "windows_service", "remove")
    }
    Write-Host "Inferra service removed."
} else {
    Write-Host "Inferra service is not installed."
}

Get-NetFirewallRule -ErrorAction SilentlyContinue | Where-Object { $_.DisplayName -like "Inferra-HTTP-*" } | ForEach-Object {
    Remove-NetFirewallRule -InputObject $_ -ErrorAction SilentlyContinue
}
