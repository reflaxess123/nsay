# Builds every sidecar in crates/ and stages binaries (plus runtime DLLs)
# next to the main nsay binary so tools::resolve_sidecar can find them
# during `tauri dev` and packaged builds.
#
# Usage:
#   .\scripts\build-sidecars.ps1                         # everything
#   .\scripts\build-sidecars.ps1 -Tool rembg             # only one tool family
#   .\scripts\build-sidecars.ps1 -Backend cpu            # only one backend
#   .\scripts\build-sidecars.ps1 -Tool rembg -Backend cpu  # one specific
#   .\scripts\build-sidecars.ps1 -Profile debug          # debug build
#
# Skips crates that don't exist yet, so adding a new sidecar is just a
# matter of creating crates/nsay-<tool>-<backend>/.

param(
    [string]$Tool = "all",
    [string]$Backend = "all",
    [ValidateSet("debug", "release")]
    [string]$Profile = "release"
)

$ErrorActionPreference = "Stop"
$root = Resolve-Path "$PSScriptRoot\.."
$cratesDir = Join-Path $root "crates"
$targetDir = Join-Path $root "target"
$stageDirs = @(
    (Join-Path $targetDir "debug"),
    (Join-Path $targetDir "release")
)

# Source for runtime DLLs that CUDA-feature ort dynamically loads at startup.
# Auto-detect the highest installed CUDA version under the standard NVIDIA
# Toolkit path; let -CudaBin override. Falls back to v13.2 only if scan
# returns nothing (preserves the old hard-coded behaviour for clean repos).
$cudaRoot = "C:\Program Files\NVIDIA GPU Computing Toolkit\CUDA"
$cudaBin = $null
if (Test-Path $cudaRoot) {
    $latest = Get-ChildItem $cudaRoot -Directory -Filter "v*" -ErrorAction SilentlyContinue |
        Sort-Object { [version]($_.Name -replace '^v','') } -Descending |
        Select-Object -First 1
    if ($latest) {
        $candidate = Join-Path $latest.FullName "bin\x64"
        if (Test-Path $candidate) { $cudaBin = $candidate }
    }
}
if (-not $cudaBin) { $cudaBin = "$cudaRoot\v13.2\bin\x64" }

# Pick cudart/cublas major suffix from the actual filenames present in
# $cudaBin instead of hard-coding cublas64_13. cudnn major still hard-
# coded to 9 (ort 2.0.0-rc.10 needs cuDNN 9.x specifically).
$cudaDlls = @()
if (Test-Path $cudaBin) {
    foreach ($pat in "cublas64_*.dll", "cublasLt64_*.dll", "cudart64_*.dll") {
        $f = Get-ChildItem $cudaBin -Filter $pat -ErrorAction SilentlyContinue |
            Select-Object -First 1
        if ($f) { $cudaDlls += $f.Name }
    }
    $cudnn = Get-ChildItem $cudaBin -Filter "cudnn64_9*.dll" -ErrorAction SilentlyContinue |
        Select-Object -First 1
    if ($cudnn) { $cudaDlls += $cudnn.Name }
}
Write-Host "CUDA bin: $cudaBin" -ForegroundColor DarkGray
Write-Host "CUDA DLLs: $($cudaDlls -join ', ')" -ForegroundColor DarkGray

function Build-One($crateName) {
    $crate = Join-Path $cratesDir $crateName
    if (-not (Test-Path $crate)) {
        Write-Host "skip: crates/$crateName not found" -ForegroundColor Yellow
        return
    }
    Write-Host ">> building $crateName ($Profile)" -ForegroundColor Cyan
    # Use the sidecar's OWN target dir (its standalone workspace) instead
    # of the parent target. This avoids cargo re-downloading the ort native
    # lib for every variant and keeps each backend's deps cleanly isolated.
    $cargoArgs = @("build", "--manifest-path", "$crate/Cargo.toml")
    if ($Profile -eq "release") { $cargoArgs += "--release" }
    & cargo @cargoArgs
    if ($LASTEXITCODE -ne 0) { throw "cargo build failed for $crateName" }

    $exe = Join-Path $crate "target\$Profile\$crateName.exe"
    if (-not (Test-Path $exe)) { throw "expected output not found: $exe" }

    # Stage in both debug and release of the parent so tauri dev (debug
    # nsay_app.exe) and packaged builds (release) both find the sidecar.
    foreach ($dst in $stageDirs) {
        if (-not (Test-Path $dst)) { New-Item -ItemType Directory -Path $dst | Out-Null }
        Copy-Item $exe (Join-Path $dst "$crateName.exe") -Force
    }

    # ort `download-binaries` extracts the onnxruntime bundle into
    # ~/AppData/Local/ort.pyke.io/dfbin/<triple>/<hash>/onnxruntime/lib/.
    # `onnxruntime.lib` is statically linked into the sidecar, but a few
    # companion DLLs are loaded **dynamically at runtime** from next to the
    # exe and must be staged:
    #   - onnxruntime_providers_shared.dll  (CUDA EP)
    #   - onnxruntime_providers_cuda.dll    (CUDA EP)
    #   - DirectML.dll                      (DirectML EP)
    # The build script writes the cache path as `cargo:rustc-link-search=
    # native=...` in target/<profile>/build/ort-sys-*/output. Pull from
    # there so we always grab the exact bundle this sidecar was linked
    # against, regardless of feature flags or version bumps.
    $ortOutFiles = Get-ChildItem -Path "$crate\target\$Profile\build" `
        -Filter "output" -Recurse -ErrorAction SilentlyContinue |
        Where-Object { $_.FullName -like "*ort-sys-*" }
    foreach ($f in $ortOutFiles) {
        $line = Select-String -Path $f.FullName -Pattern "rustc-link-search=native=.*onnxruntime\\lib" |
            Select-Object -Last 1
        if (-not $line) { continue }
        $libDir = ($line.Line -split "rustc-link-search=native=", 2)[1]
        if (-not (Test-Path $libDir)) { continue }
        Get-ChildItem -Path $libDir -Filter "*.dll" | ForEach-Object {
            foreach ($dst in $stageDirs) {
                Copy-Item $_.FullName (Join-Path $dst $_.Name) -Force
            }
        }
        break
    }

    if ($crateName -like "*-cuda") {
        Stage-CudaDlls
    }
}

function Stage-CudaDlls {
    if (-not (Test-Path $cudaBin)) {
        Write-Warning "CUDA bin dir not found at $cudaBin — runtime DLLs not staged"
        return
    }
    foreach ($dst in $stageDirs) {
        foreach ($dll in $cudaDlls) {
            $src = Join-Path $cudaBin $dll
            if (Test-Path $src) {
                Copy-Item $src (Join-Path $dst $dll) -Force
            } else {
                Write-Warning "missing: $src (ok if not using CUDA)"
            }
        }
    }
}

# Enumerate crates/nsay-<tool>-<backend> that match the filters. Exclude
# `nsay-<tool>-lib` shared-pipeline crates — they're library deps of the
# bin crates, not buildable sidecars themselves (no [[bin]], no exe out).
$candidates = Get-ChildItem $cratesDir -Directory -ErrorAction SilentlyContinue |
    Where-Object { $_.Name -like "nsay-*-*" -and $_.Name -notlike "*-lib" }

foreach ($d in $candidates) {
    $parts = $d.Name -split "-"
    if ($parts.Count -lt 3) { continue }
    $cTool    = $parts[1]
    $cBackend = $parts[2..($parts.Count-1)] -join "-"
    if ($Tool    -ne "all" -and $cTool    -ne $Tool)    { continue }
    if ($Backend -ne "all" -and $cBackend -ne $Backend) { continue }
    Build-One $d.Name
}

Write-Host "done." -ForegroundColor Green
