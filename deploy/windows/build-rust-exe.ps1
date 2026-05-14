#requires -Version 5.1
param(
    [string]$Cargo = "cargo",
    [string]$Target = "",
    [switch]$CopyUiBundle,
    [switch]$CopyPythonWorker
)

$ErrorActionPreference = 'Stop'

$repoRoot = Resolve-Path (Join-Path $PSScriptRoot "..\..")
$rustRoot = Join-Path $repoRoot "src"
$distDir = Join-Path $repoRoot "dist"
$uiDist = Join-Path $repoRoot "src\web\ui_dist"
$runtimeAssets = Join-Path $distDir "runtime-assets"
$targetDir = Join-Path $repoRoot "build\rust-target"

function Publish-InferraBinary {
    param(
        [Parameter(Mandatory = $true)][string]$SourcePath,
        [Parameter(Mandatory = $true)][string]$PreferredPath,
        [Parameter(Mandatory = $true)][string]$ArtifactLabel
    )

    try {
        Copy-Item $SourcePath $PreferredPath -Force
        return $PreferredPath
    }
    catch {
        $timestamp = Get-Date -Format "yyyyMMdd-HHmmss"
        $fallbackName = "{0}-{1}.exe" -f ([System.IO.Path]::GetFileNameWithoutExtension($PreferredPath)), $timestamp
        $fallbackPath = Join-Path ([System.IO.Path]::GetDirectoryName($PreferredPath)) $fallbackName
        Copy-Item $SourcePath $fallbackPath -Force
        Write-Warning "$ArtifactLabel could not overwrite $PreferredPath because it is locked. Wrote a fresh artifact to $fallbackPath instead."
        return $fallbackPath
    }
}

Push-Location $rustRoot
try {
    $cargoArgs = @("build", "--release", "-p", "inferra-cli", "--target-dir", $targetDir)
    if ($Target) {
        $cargoArgs += @("--target", $Target)
    }
    & $Cargo @cargoArgs
}
finally {
    Pop-Location
}

New-Item -ItemType Directory -Force -Path $distDir | Out-Null
if ($Target) {
    $exePath = Join-Path $targetDir "$Target\release\inferra.exe"
}
else {
    $exePath = Join-Path $targetDir "release\inferra.exe"
}

$nativeExe = Join-Path $distDir "inferra-rust.exe"
$publishedExe = Publish-InferraBinary -SourcePath $exePath -PreferredPath $nativeExe -ArtifactLabel "Native artifact"

$stableExe = Join-Path $distDir "inferra.exe"
try {
    Copy-Item $exePath $stableExe -Force
}
catch {
    Write-Warning "Could not overwrite $stableExe because it is locked. Native artifact remains available at $publishedExe"
}

if (-not (Test-Path $uiDist)) {
    throw "UI bundle not found at $uiDist. Run npm run build in src\web\frontend before building the runtime package."
}

$uiTarget = Join-Path $runtimeAssets "ui_dist"
New-Item -ItemType Directory -Force -Path $uiTarget | Out-Null
Remove-Item (Join-Path $uiTarget "*") -Recurse -Force -ErrorAction SilentlyContinue
Copy-Item (Join-Path $uiDist "*") $uiTarget -Recurse -Force

if ($CopyPythonWorker) {
    Write-Warning "CopyPythonWorker is ignored. The Rust build is self-contained and does not bundle the source tree or legacy Python runtime."
}

Write-Host "Primary artifact: $publishedExe"
