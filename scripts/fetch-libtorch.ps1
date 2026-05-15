# Fetches the libtorch C++ runtime that nsay-vidsr-* sidecars (PLAN.md F4)
# link against via tch-rs. CUDA build by default; pass -Cpu for a much
# smaller CPU-only fallback.
#
# Usage:
#   .\scripts\fetch-libtorch.ps1                # CUDA 12.4 release, ~2.3 GB
#   .\scripts\fetch-libtorch.ps1 -Cpu           # CPU only, ~180 MB
#   .\scripts\fetch-libtorch.ps1 -Version 2.5.0 # pin a different libtorch
#
# Why bundled, not system-installed:
#   tch-rs hard-links specific torch versions, and a system-wide install
#   on a dev box can drift. Bundling a known-good version next to the
#   sidecar means CI / packaged builds reproduce locally.
#
# Output staging:
#   src-tauri\binaries\libtorch\<lib|bin|include|share>
#   The directory layout mirrors the upstream zip — build.rs of tch
#   reads LIBTORCH and walks lib/ for .lib + bin/ for runtime DLLs.

param(
    [switch]$Cpu,
    [string]$Version = "2.5.0",
    [string]$CudaTag = "cu124"
)

$ErrorActionPreference = "Stop"
$root = Resolve-Path "$PSScriptRoot\.."
$dest = Join-Path $root "src-tauri\binaries\libtorch"

$flavor = if ($Cpu) { "cpu" } else { $CudaTag }
$zipName = "libtorch-win-shared-with-deps-$Version+$flavor.zip"
$url = "https://download.pytorch.org/libtorch/$flavor/$zipName"

Write-Host "libtorch: $Version + $flavor" -ForegroundColor Cyan
Write-Host "url:      $url" -ForegroundColor DarkGray
Write-Host "dest:     $dest" -ForegroundColor DarkGray

if (Test-Path $dest) {
    Write-Host "warn: $dest already exists. Delete it first to re-fetch." -ForegroundColor Yellow
    return
}

$tmpDir = Join-Path $env:TEMP "nsay-libtorch"
New-Item -ItemType Directory -Force -Path $tmpDir | Out-Null
$zipPath = Join-Path $tmpDir $zipName

if (-not (Test-Path $zipPath)) {
    Write-Host ">> downloading libtorch (~$(if ($Cpu) { '180 MB' } else { '2.3 GB' }))..." -ForegroundColor Cyan
    # Use BITS for resumability on flaky connections; falls back to
    # Invoke-WebRequest if BITS isn't present (e.g. PowerShell 7 Core
    # on a non-domain box without BitsTransfer module).
    try {
        Import-Module BitsTransfer -ErrorAction Stop
        Start-BitsTransfer -Source $url -Destination $zipPath
    } catch {
        Invoke-WebRequest -Uri $url -OutFile $zipPath -UseBasicParsing
    }
}

Write-Host ">> extracting to $dest" -ForegroundColor Cyan
$extractTmp = Join-Path $tmpDir "extract"
if (Test-Path $extractTmp) { Remove-Item -Recurse -Force $extractTmp }
Expand-Archive -Path $zipPath -DestinationPath $extractTmp

# Zip layout: extract\libtorch\{lib,bin,include,share,...}
$inner = Join-Path $extractTmp "libtorch"
if (-not (Test-Path $inner)) {
    throw "Unexpected zip layout: no 'libtorch' folder under extract root."
}
New-Item -ItemType Directory -Force -Path (Split-Path $dest -Parent) | Out-Null
Move-Item $inner $dest

Write-Host "done. Set LIBTORCH for tch-rs builds:" -ForegroundColor Green
Write-Host "  `$env:LIBTORCH = '$dest'" -ForegroundColor Green
Write-Host "  `$env:Path = `"`$env:LIBTORCH\lib;`$env:Path`""           -ForegroundColor Green
