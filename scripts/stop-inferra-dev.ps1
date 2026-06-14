# Stop Inferra Windows service, inferra.exe processes, and the Vite dev server when present.
$ErrorActionPreference = "Stop"
$Root = Split-Path -Parent $PSScriptRoot
$wm = Join-Path $Root "deploy\windows\InferraWindows.psm1"

if (Test-Path $wm) {
    Import-Module -Name $wm -Force
    Stop-InferraWindowsExecutionLocks -ServiceStopTimeoutSec 120
}

foreach ($port in @(5173, 7433, 7434, 7435, 7436, 7437)) {
    $lines = netstat -ano | Select-String "127.0.0.1:$port\s+.*LISTENING"
    foreach ($line in $lines) {
        $procId = ($line -split '\s+')[-1]
        if ($procId -match '^\d+$') {
            Write-Host "Stopping PID $procId listening on port $port"
            Stop-Process -Id ([int]$procId) -Force -ErrorAction SilentlyContinue
        }
    }
}

Write-Host "Inferra dev processes stopped."
