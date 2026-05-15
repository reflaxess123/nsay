# Fetch NVIDIA TensorRT runtime DLLs and stage them where the
# tensorrt-feature `ort` sidecars look them up at runtime.
#
# Why a temp venv instead of `pip download` like fetch-cuda-runtime.ps1:
# NVIDIA publishes `tensorrt-cu12-libs` on PyPI as an sdist, not a wheel.
# The actual DLLs are downloaded by a post-install hook inside their setup.py
# and end up in `<env>/Lib/site-packages/tensorrt_libs/`. So we have to
# actually `pip install` (not just `pip download`) to materialise them.
#
# Output:
#   src-tauri/binaries/runtime/   ← canonical place (Tauri bundles this)
#   target/debug/                 ← so `tauri dev` finds them
#   target/release/               ← so unbundled `cargo run --release` works
#
# DLL inventory (TRT 10.x):
#   nvinfer_10.dll                              — main runtime (~250 MB)
#   nvinfer_plugin_10.dll                       — built-in plugins
#   nvonnxparser_10.dll                         — ONNX → TRT graph parser
#   nvinfer_builder_resource_ptx_10.dll         — generic PTX fallback
#   nvinfer_builder_resource_sm{NN}_10.dll      — per-arch cubin (~200-400 MB each)
#     sm75 = Turing  (RTX 20xx)
#     sm80 = Ampere  (A100)
#     sm86 = Ampere  (RTX 30xx — including this user's 3080 Ti)
#     sm89 = Ada     (RTX 40xx)
#     sm90 = Hopper  (H100)
#     sm100/sm120    = Blackwell+ (future)
#
# By default we stage *only* the SMs the user asks for + always-needed
# DLLs. Pass -StageAllSms to keep them all (~2 GB extra). Default keeps
# just sm86 + ptx fallback for the developer machine (3080 Ti).
#
# Re-runnable: skips already-downloaded venv if temp venv exists.

param(
    [string[]]$Sms = @("sm86"),    # which SM cubins to stage (sm86 = RTX 30xx)
    [switch]$StageAllSms,          # keep every SM cubin (≈ +2 GB)
    [string]$TempVenv = "$env:LOCALAPPDATA\nsay\trt-extract-venv",
    [switch]$Force                 # nuke temp venv and reinstall
)

$ErrorActionPreference = "Stop"
$root = Resolve-Path "$PSScriptRoot\.."

function Need-Cmd($name, $hint) {
    $cmd = Get-Command $name -ErrorAction SilentlyContinue
    if (-not $cmd) {
        Write-Error "$name not found in PATH. $hint"
        exit 1
    }
    return $cmd.Source
}

# uv is required — same bootstrap pattern as setup-seedvr2.ps1 (which we
# removed but the pattern is sound). uv handles managed Python, so we
# don't need a system Python at all.
$uv = Get-Command "uv" -ErrorAction SilentlyContinue
if (-not $uv) {
    Write-Host "==> installing uv (https://astral.sh/uv)" -ForegroundColor Cyan
    powershell -ExecutionPolicy ByPass -c "irm https://astral.sh/uv/install.ps1 | iex"
    if ($LASTEXITCODE -ne 0) { throw "uv install failed" }
    $env:Path = [Environment]::GetEnvironmentVariable("Path", "User") + ";" + [Environment]::GetEnvironmentVariable("Path", "Machine")
    $uv = Get-Command "uv" -ErrorAction SilentlyContinue
    if (-not $uv) {
        Write-Error "uv installed but not on PATH. Open a new terminal and re-run this script."
        exit 1
    }
}

# --- Step 1: temp venv + tensorrt-cu12 install --------------------------

if ($Force -and (Test-Path $TempVenv)) {
    Write-Host "==> -Force: removing temp venv $TempVenv" -ForegroundColor Cyan
    Remove-Item -Recurse -Force $TempVenv
}

$venvPython = Join-Path $TempVenv "Scripts\python.exe"
if (-not (Test-Path $venvPython)) {
    Write-Host "==> creating temp venv at $TempVenv (uv-managed Python 3.11)" -ForegroundColor Cyan
    & uv venv --python 3.11 $TempVenv
    if ($LASTEXITCODE -ne 0) { throw "uv venv failed" }
}

# tensorrt-cu12 == 10.x: pinned major to match the ort tensorrt EP build
# manifest. If you bump ort, re-check what TRT major it's linked against.
Write-Host "==> installing tensorrt-cu12 (~2 GB download, may take 3-5 min)" -ForegroundColor Cyan
& uv pip install --python $venvPython tensorrt-cu12
if ($LASTEXITCODE -ne 0) { throw "tensorrt-cu12 install failed" }

$libsDir = Join-Path $TempVenv "Lib\site-packages\tensorrt_libs"
if (-not (Test-Path $libsDir)) {
    throw "tensorrt_libs/ not found at $libsDir — install layout may have changed"
}

# --- Step 2: collect DLLs (filter SMs) ----------------------------------

$allDlls = Get-ChildItem -Path $libsDir -Filter "*.dll"
Write-Host "==> $($allDlls.Count) DLLs total in tensorrt_libs/" -ForegroundColor DarkGray

$wanted = $allDlls | Where-Object {
    $name = $_.Name
    if ($name -notmatch '_builder_resource_sm\d+_') {
        # Always-needed DLLs: nvinfer_10, nvinfer_plugin_10, nvonnxparser_10,
        # plus the generic PTX fallback. None of these have an sm tag.
        return $true
    }
    if ($StageAllSms) { return $true }
    # SM-specific cubin: keep only the SMs the user asked for.
    foreach ($sm in $Sms) {
        if ($name -match "_${sm}_") { return $true }
    }
    return $false
}

if (-not $wanted -or $wanted.Count -eq 0) {
    throw "Filter dropped every DLL — check -Sms parameter (got: $($Sms -join ','))"
}

$dropped = $allDlls.Count - $wanted.Count
if ($dropped -gt 0) {
    Write-Host "    skipping $dropped SM-specific cubin(s) (use -StageAllSms to keep all)" -ForegroundColor DarkGray
}

# --- Step 3: stage to three target dirs ---------------------------------

$stageDirs = @(
    (Join-Path $root "src-tauri\binaries\runtime"),
    (Join-Path $root "target\debug"),
    (Join-Path $root "target\release")
) | ForEach-Object { (New-Item -ItemType Directory -Path $_ -Force).FullName }

Write-Host "==> staging $($wanted.Count) DLLs into:" -ForegroundColor Cyan
foreach ($d in $stageDirs) { Write-Host "    - $d" }

$totalBytes = 0L
foreach ($dll in $wanted) {
    foreach ($dst in $stageDirs) {
        $out = Join-Path $dst $dll.Name
        if (-not $Force -and (Test-Path $out) -and ((Get-Item $out).Length -eq $dll.Length)) {
            continue
        }
        Copy-Item $dll.FullName $out -Force
    }
    $totalBytes += $dll.Length
    $sizeStr = "{0,7:N1} MB" -f ($dll.Length / 1MB)
    Write-Host ("    {0,-50}  {1}" -f $dll.Name, $sizeStr)
}

$totalMb = [math]::Round($totalBytes / 1MB, 1)
Write-Host ""
Write-Host "done. staged $($wanted.Count) DLLs, $totalMb MB per location." -ForegroundColor Green
Write-Host ""
Write-Host "Temp venv kept at $TempVenv for re-runs (rm to free $('{0:N1}' -f ((Get-ChildItem $TempVenv -Recurse -File -ErrorAction SilentlyContinue | Measure-Object Length -Sum).Sum / 1GB)) GB)." -ForegroundColor DarkGray
Write-Host ""
Write-Host "Next: rebuild nsay-upscale-cuda with the tensorrt feature:" -ForegroundColor Yellow
Write-Host "  cd crates\nsay-upscale-cuda; cargo build --release"
