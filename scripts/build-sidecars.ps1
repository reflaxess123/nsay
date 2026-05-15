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
# Adjust if CUDA installs elsewhere.
$cudaBin = "C:\Program Files\NVIDIA GPU Computing Toolkit\CUDA\v13.2\bin\x64"
$cudaDlls = @("cublas64_13.dll", "cublasLt64_13.dll", "cudart64_13.dll", "cudnn64_9.dll")

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

# Enumerate crates/nsay-<tool>-<backend> that match the filters.
$candidates = Get-ChildItem $cratesDir -Directory -ErrorAction SilentlyContinue |
    Where-Object { $_.Name -like "nsay-*-*" }

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
