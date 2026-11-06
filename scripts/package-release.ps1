#Requires -Version 5.1
<#
.SYNOPSIS
    Builds the CLI + GUI in release mode and stages ONLY the files needed to run them.

.DESCRIPTION
    Produces dist/pgtools-<Version>-win64 containing:
        pgtools.exe       - unified CLI
        pgtools-gui.exe   - Tauri desktop app (frontend assets are embedded in the exe)
        README.md, LICENSE
        (optional) oo2core_*.dll - only if you pass -OodleDll or one sits in the repo root

    Nothing else from target/release is copied: no .pdb, .d dep-info files,
    deps/, build/, .fingerprint/, incremental/, wix/, nsis/ staging dirs.

.PARAMETER OutDir
    Staging root, relative to the repo root. Default: dist

.PARAMETER Version
    Version string used in the folder/zip name. Default: 0.1.0

.PARAMETER OodleDll
    Optional path to oo2core_X_win64.dll to ship alongside the exes.
    (It is not redistributable, so this is opt-in.)

.PARAMETER SkipFrontend
    Skip `npm run build` for the GUI frontend (reuses the existing crates/pgtools-gui/dist).

.PARAMETER SkipBuild
    Skip cargo build (repackage the exes already in target/release).

.PARAMETER Zip
    Also create dist/pgtools-<Version>-win64.zip next to the staged folder.

.EXAMPLE
    powershell -ExecutionPolicy Bypass -File scripts\package-release.ps1 -Zip
#>
[CmdletBinding()]
param(
    [string]$OutDir = "dist",
    [string]$Version = "0.1.0",
    [string]$OodleDll = "",
    [switch]$SkipFrontend,
    [switch]$SkipBuild,
    [switch]$Zip
)

$ErrorActionPreference = "Stop"

$root       = Split-Path -Parent $PSScriptRoot
$guiDir     = Join-Path $root "crates\pgtools-gui"
$releaseDir = Join-Path $root "target\release"
$stageRoot  = Join-Path $root $OutDir
$stage      = Join-Path $stageRoot "pgtools-$Version-win64"

# --- locate cargo ---------------------------------------------------------
$cargo = (Get-Command cargo -ErrorAction SilentlyContinue).Source
if (-not $cargo) {
    $cargo = Join-Path $env:USERPROFILE ".cargo\bin\cargo.exe"
}
if (-not (Test-Path $cargo)) {
    throw "cargo not found on PATH or in %USERPROFILE%\.cargo\bin"
}

# --- 1. frontend ----------------------------------------------------------
if (-not $SkipFrontend) {
    Write-Host "==> Building GUI frontend" -ForegroundColor Cyan
    Push-Location $guiDir
    try {
        if (-not (Test-Path "node_modules")) {
            & npm install
            if ($LASTEXITCODE -ne 0) { throw "npm install failed" }
        }
        & npm run build
        if ($LASTEXITCODE -ne 0) { throw "npm run build failed" }
    }
    finally {
        Pop-Location
    }
}

# --- 2. cargo release build ----------------------------------------------
if (-not $SkipBuild) {
    Write-Host "==> cargo build --release (cli + gui)" -ForegroundColor Cyan
    Push-Location $root
    try {
        & $cargo build --release --locked -p pgtools-cli -p pgtools-gui `
            --features pgtools-gui/custom-protocol
        if ($LASTEXITCODE -ne 0) { throw "cargo build failed" }
    }
    finally {
        Pop-Location
    }
}

# --- 3. stage only the runtime files -------------------------------------
$wanted = @("pgtools.exe", "pgtools-gui.exe")
foreach ($exe in $wanted) {
    $src = Join-Path $releaseDir $exe
    if (-not (Test-Path $src)) { throw "Missing $src - run without -SkipBuild first" }
}

if (Test-Path $stage) { Remove-Item $stage -Recurse -Force }
New-Item -ItemType Directory -Force -Path $stage | Out-Null

foreach ($exe in $wanted) {
    Write-Host "    + $exe"
    Copy-Item (Join-Path $releaseDir $exe) $stage -Force
}
foreach ($doc in @("README.md", "LICENSE")) {
    $src = Join-Path $root $doc
    if (Test-Path $src) {
        Write-Host "    + $doc"
        Copy-Item $src $stage -Force
    }
}

# Optional Oodle runtime (user-supplied, not redistributable)
$oodleSrc = $null
if ($OodleDll) {
    if (-not (Test-Path $OodleDll)) { throw "Oodle DLL not found: $OodleDll" }
    $oodleSrc = (Resolve-Path $OodleDll).Path
}
else {
    $found = Get-ChildItem $root -Filter "oo2core_*_win64.dll" -File -ErrorAction SilentlyContinue |
             Select-Object -First 1
    if ($found) { $oodleSrc = $found.FullName }
}
if ($oodleSrc) {
    Write-Host "    + $(Split-Path -Leaf $oodleSrc) (optional Oodle runtime)"
    Copy-Item $oodleSrc $stage -Force
}

# Belt and braces: never let a PDB or dep-info file reach the package
Get-ChildItem $stage -Recurse -File |
    Where-Object { $_.Extension -in @(".pdb", ".d", ".rlib", ".rmeta", ".lib", ".exp", ".ilk") } |
    Remove-Item -Force

# link.exe always writes <bin>.pdb next to the exe on MSVC. It is not needed to
# run anything, so keep target/release tidy as well.
Get-ChildItem $releaseDir -Filter "*.pdb" -File -ErrorAction SilentlyContinue |
    Where-Object { $_.BaseName -in @("pgtools", "pgtools_gui") } |
    ForEach-Object {
        Write-Host "    - $($_.Name) (not needed at runtime)"
        Remove-Item $_.FullName -Force
    }

# --- 4. report / zip -----------------------------------------------------
Write-Host ""
Write-Host "Staged in $stage" -ForegroundColor Green
Get-ChildItem $stage | ForEach-Object {
    "    {0,-28} {1,8:N2} MB" -f $_.Name, ($_.Length / 1MB)
}

if ($Zip) {
    $zipPath = Join-Path $stageRoot "pgtools-$Version-win64.zip"
    if (Test-Path $zipPath) { Remove-Item $zipPath -Force }
    Compress-Archive -Path (Join-Path $stage "*") -DestinationPath $zipPath
    Write-Host ""
    Write-Host "Archive: $zipPath" -ForegroundColor Green
}
