# Post-build step: moves the built app out of target/ as <app-name>_<version>.exe.
#
# This is a TEMPLATE (tracked in git). Each PC uses its own copy:
#   Copy-Item scripts\post-build.example.ps1 scripts\post-build.ps1
# then edit $OutputDir below. scripts/post-build.ps1 is git-ignored.
#
# Run automatically by:  pnpm release

param(
    # Cargo profile folder under target/ (release | debug)
    [string]$BuildProfile = "release"
)

$ErrorActionPreference = "Stop"

# ---- Per-PC settings -------------------------------------------------------
# Where the final exe goes. Absolute path, or relative to the repo root.
$OutputDir = "release"
# ---------------------------------------------------------------------------

$root = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
if (-not [IO.Path]::IsPathRooted($OutputDir)) {
    $OutputDir = Join-Path $root $OutputDir
}

# App name and version come from tauri.conf.json.
$conf = Get-Content (Join-Path $root "src-tauri\tauri.conf.json") -Raw | ConvertFrom-Json
$appName = $conf.productName
$version = $conf.version

# The raw exe in target/ is named after mainBinaryName, else the Cargo package name.
$binName = $conf.mainBinaryName
if (-not $binName) {
    $cargoToml = Get-Content (Join-Path $root "src-tauri\Cargo.toml") -Raw
    if ($cargoToml -notmatch '(?ms)^\[package\].*?^name\s*=\s*"([^"]+)"') {
        throw "Cannot read the package name from src-tauri\Cargo.toml"
    }
    $binName = $Matches[1]
}

$buildDir = Join-Path $root "target\$BuildProfile"
$srcExe = Join-Path $buildDir "$binName.exe"
if (-not (Test-Path -LiteralPath $srcExe)) {
    throw "Build output not found: $srcExe (run 'pnpm tauri build' first)"
}

New-Item -ItemType Directory -Force -Path $OutputDir | Out-Null
$destExe = Join-Path $OutputDir "${appName}_${version}.exe"
Move-Item -LiteralPath $srcExe -Destination $destExe -Force

# The app needs seaf-daemon.exe and its DLLs in binaries\ next to the exe.
$srcBin = Join-Path $buildDir "binaries"
if (Test-Path -LiteralPath $srcBin) {
    $destBin = Join-Path $OutputDir "binaries"
    New-Item -ItemType Directory -Force -Path $destBin | Out-Null
    Copy-Item -Path (Join-Path $srcBin "*") -Destination $destBin -Recurse -Force
} else {
    Write-Warning "No binaries\ folder in $buildDir; seaf-daemon will be missing."
}

Write-Host "Output: $destExe"
