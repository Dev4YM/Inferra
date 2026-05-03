$Python = "python"
$ErrorActionPreference = "Stop"
$existing = Get-Service -Name "Inferra" -ErrorAction SilentlyContinue
if ($existing) {
    Stop-Service Inferra -ErrorAction SilentlyContinue
    & $Python -m windows_service remove | Out-Null
    Write-Host "Inferra service removed."
} else {
    Write-Host "Inferra service is not installed."
}
