# One-time setup for the SeedVR2 native VSR runner.
#
# SeedVR2 (ICLR 2026, ByteDance) — "One-Step Video Restoration via Diffusion
# Adversarial Post-Training". One-step diffusion transformer with native
# temporal awareness. Uses the community-maintained ComfyUI repo
# (numz/ComfyUI-SeedVR2_VideoUpscaler, 2.4k stars) which exposes a
# standalone CLI (inference_cli.py) — no ComfyUI runtime needed.
#
# Why this beats every other diffusion VSR we've tried:
#   - flash_attn is OPTIONAL (graceful fallback to PyTorch SDPA on Windows
#     where flash_attn wheels don't exist for new CUDA versions).
#   - No xformers, mmcv, triton, sageattention — all the typical Windows
#     compile killers are absent from requirements.txt.
#   - GGUF Q4/Q8 quantization shrinks the 7B model from 16.5 GB FP16 down
#     to 4.76 GB Q4 — 12 GB GPUs can run the 7B Sharp variant headroom-free.
#   - inference_cli.py takes --input <file>, --output <path>, --resolution N,
#     --batch_size {1,5,9,13...} (4n+1) — perfect for our sidecar pattern.
#
# Layout:
#   %APPDATA%\nsay\runners\seedvr2\
#   ├── repo\               (numz/ComfyUI-SeedVR2_VideoUpscaler clone)
#   ├── .venv\              (uv-managed venv, Python 3.12)
#   └── repo\models\        (VAE + transformer weights from HuggingFace)
#
# Default model picks for 12 GB VRAM (RTX 3080 Ti):
#   - ema_vae_fp16.safetensors                (501 MB, mandatory VAE)
#   - seedvr2_ema_7b_sharp_fp8_e4m3fn.safetensors (8.24 GB, best quality)
# Fallback for tighter VRAM via -Lite:
#   - seedvr2_ema_3b_fp8_e4m3fn.safetensors   (3.39 GB)
#
# Total disk: ~9 GB weights + ~5 GB venv (torch + cuDNN bundled) ≈ 15 GB.

param(
    [switch]$Force,        # nuke and rebuild venv + re-clone + redownload
    [switch]$SkipWeights,  # skip HF download (use existing files)
    [switch]$Lite,         # use 3B model instead of 7B (for <12 GB GPUs)
    [string]$CudaIndex = "https://download.pytorch.org/whl/cu124"
)

$ErrorActionPreference = "Stop"

function Need-Cmd($name, $hint) {
    $cmd = Get-Command $name -ErrorAction SilentlyContinue
    if (-not $cmd) {
        Write-Error "$name not found in PATH. $hint"
        exit 1
    }
    return $cmd.Source
}

# --- Pre-flight ---------------------------------------------------------

Write-Host "==> pre-flight checks" -ForegroundColor Cyan

Need-Cmd "git" "Install Git for Windows (https://git-scm.com/download/win)." | Out-Null

# uv is the fast package installer/venv manager from astral. We bootstrap
# it on demand because it's a single 30 MB install and avoids needing the
# user to have a system Python at all (uv can manage Python itself).
$uv = Get-Command "uv" -ErrorAction SilentlyContinue
if (-not $uv) {
    Write-Host "==> installing uv (https://astral.sh/uv)" -ForegroundColor Cyan
    powershell -ExecutionPolicy ByPass -c "irm https://astral.sh/uv/install.ps1 | iex"
    if ($LASTEXITCODE -ne 0) { throw "uv install failed" }
    # Refresh PATH so uv becomes visible without a shell restart.
    $env:Path = [Environment]::GetEnvironmentVariable("Path", "User") + ";" + [Environment]::GetEnvironmentVariable("Path", "Machine")
    $uv = Get-Command "uv" -ErrorAction SilentlyContinue
    if (-not $uv) {
        Write-Error "uv installed but not on PATH. Open a new terminal and re-run this script."
        exit 1
    }
}
Write-Host "    uv: $($uv.Source)" -ForegroundColor DarkGray

# --- Paths --------------------------------------------------------------

$root    = Join-Path $env:APPDATA "nsay\runners\seedvr2"
$repo    = Join-Path $root "repo"
$venv    = Join-Path $root ".venv"
$models  = Join-Path $repo "models"
$pyenv   = Join-Path $venv "Scripts\python.exe"
$repoUrl = "https://github.com/numz/ComfyUI-SeedVR2_VideoUpscaler.git"

Write-Host ""
Write-Host "    root    : $root" -ForegroundColor DarkGray
Write-Host "    repo    : $repo" -ForegroundColor DarkGray
Write-Host "    venv    : $venv" -ForegroundColor DarkGray
Write-Host "    weights : $models" -ForegroundColor DarkGray
Write-Host "    profile : $(if ($Lite) {'3B FP8 (lite, ~3.4 GB)'} else {'7B Sharp FP8 (~8.2 GB)'})" -ForegroundColor DarkGray
Write-Host ""

if (-not (Test-Path $root)) { New-Item -ItemType Directory -Force -Path $root | Out-Null }

# --- Step 1: clone repo ------------------------------------------------

if ($Force -and (Test-Path $repo)) {
    Write-Host "==> -Force: removing existing clone $repo" -ForegroundColor Cyan
    Remove-Item -Recurse -Force $repo
}
if (Test-Path $repo) {
    Write-Host "==> updating existing SeedVR2 clone" -ForegroundColor Cyan
    & git -C $repo fetch --depth 1 origin main
    if ($LASTEXITCODE -ne 0) { throw "git fetch failed" }
    & git -C $repo reset --hard origin/main
    if ($LASTEXITCODE -ne 0) { throw "git reset failed" }
} else {
    Write-Host "==> git clone $repoUrl" -ForegroundColor Cyan
    & git clone --depth 1 $repoUrl $repo
    if ($LASTEXITCODE -ne 0) { throw "git clone failed" }
}

# --- Step 2: venv ------------------------------------------------------

if ($Force -and (Test-Path $venv)) {
    Write-Host "==> -Force: removing existing venv $venv" -ForegroundColor Cyan
    Remove-Item -Recurse -Force $venv
}
if (-not (Test-Path $venv)) {
    Write-Host "==> creating venv at $venv (uv will manage Python 3.12)" -ForegroundColor Cyan
    # uv venv with --python 3.12 will download a managed Python build if
    # the system doesn't have one — same outcome as python-build-standalone
    # but managed by uv so we don't need a separate fetch script.
    & uv venv --python 3.12 $venv
    if ($LASTEXITCODE -ne 0) { throw "uv venv failed" }
}
if (-not (Test-Path $pyenv)) {
    throw "venv python missing at $pyenv (creation may have failed silently)"
}

# --- Step 3: torch first (forces CUDA index, then requirements.txt sees
#     it as already-satisfied so it doesn't pull the CPU wheel) ----------

Write-Host "==> installing torch + torchvision from $CudaIndex" -ForegroundColor Cyan
Write-Host "    ~2.5 GB download (cuDNN 9 bundled in wheel)" -ForegroundColor DarkGray
& uv pip install --python $pyenv --index-url $CudaIndex torch torchvision
if ($LASTEXITCODE -ne 0) { throw "torch install failed (cu124 index unreachable?)" }

# --- Step 4: rest of SeedVR2 deps --------------------------------------

Write-Host "==> installing SeedVR2 requirements" -ForegroundColor Cyan
$reqFile = Join-Path $repo "requirements.txt"
if (-not (Test-Path $reqFile)) { throw "requirements.txt not found at $reqFile" }
& uv pip install --python $pyenv -r $reqFile
if ($LASTEXITCODE -ne 0) { throw "requirements.txt install failed" }

# huggingface_hub for the snapshot_download call below — not in requirements
# but bundled with diffusers as a transitive dep, so this is just a safety net.
& uv pip install --python $pyenv "huggingface_hub>=0.25"
if ($LASTEXITCODE -ne 0) { throw "huggingface_hub install failed" }

# --- Step 5: weights ---------------------------------------------------

if (-not $SkipWeights) {
    if (-not (Test-Path $models)) { New-Item -ItemType Directory -Force -Path $models | Out-Null }

    # Pick model based on -Lite flag.
    if ($Lite) {
        $modelFile = "seedvr2_ema_3b_fp8_e4m3fn.safetensors"
        $modelLabel = "3B FP8 (~3.4 GB)"
    } else {
        $modelFile = "seedvr2_ema_7b_sharp_fp8_e4m3fn.safetensors"
        $modelLabel = "7B Sharp FP8 (~8.2 GB)"
    }
    $vaeFile = "ema_vae_fp16.safetensors"

    $vaePath   = Join-Path $models $vaeFile
    $modelPath = Join-Path $models $modelFile

    # huggingface_hub.hf_hub_download is the safer path than urlretrieve
    # because it handles redirects + LFS pointers + retries automatically.
    $dl = @"
from huggingface_hub import hf_hub_download
import os, shutil
target_dir = r'$models'
def fetch(filename):
    out = os.path.join(target_dir, filename)
    if os.path.exists(out) and not $($Force.ToString().ToLower()):
        print(f'    {filename}: already present, skipping')
        return
    print(f'    {filename}: downloading...')
    p = hf_hub_download(repo_id='numz/SeedVR2_comfyUI', filename=filename, local_dir=target_dir)
    # hf_hub_download may use symlinks; resolve to a real file copy to be
    # robust against later cache cleanups.
    if os.path.islink(p):
        real = os.path.realpath(p)
        os.unlink(p)
        shutil.copyfile(real, p)
    print(f'    {filename}: OK')

fetch('$vaeFile')
fetch('$modelFile')
"@
    Write-Host "==> downloading weights ($modelLabel + VAE) from HuggingFace" -ForegroundColor Cyan
    & $pyenv -c $dl
    if ($LASTEXITCODE -ne 0) { throw "weight download failed" }
} else {
    Write-Host "==> skipping weights (-SkipWeights)" -ForegroundColor Yellow
}

# --- Step 6: smoke test ------------------------------------------------

Write-Host ""
Write-Host "==> smoke test" -ForegroundColor Cyan
$smoke = @"
import sys
print('python    :', sys.executable)
import torch
print('torch     :', torch.__version__, '| cuda available:', torch.cuda.is_available())
if torch.cuda.is_available():
    p = torch.cuda.get_device_properties(0)
    print('cuda dev 0:', p.name, '|', round(p.total_memory / 1024**3, 1), 'GB')
import diffusers, peft, safetensors, einops
print('diffusers :', diffusers.__version__)
print('peft      :', peft.__version__)
print('OK — SeedVR2 deps importable, CUDA visible')
"@
& $pyenv -c $smoke
if ($LASTEXITCODE -ne 0) {
    Write-Warning "Smoke test failed — deps installed but something doesn't import. See above."
    exit 1
}

# Verify inference_cli.py exists and accepts --help (catches catastrophic
# repo-side regressions without actually running inference).
Write-Host ""
Write-Host "==> verifying inference_cli.py --help" -ForegroundColor Cyan
$cli = Join-Path $repo "inference_cli.py"
if (-not (Test-Path $cli)) {
    Write-Warning "inference_cli.py not found at $cli — repo layout may have changed."
    Write-Warning "Check the repo manually: git log -1 in $repo"
    exit 1
}
Push-Location $repo
try {
    & $pyenv inference_cli.py --help 2>&1 | Select-Object -First 30
    if ($LASTEXITCODE -ne 0) {
        Write-Warning "inference_cli.py --help failed. See above."
        exit 1
    }
} finally {
    Pop-Location
}

# --- Done --------------------------------------------------------------

Write-Host ""
Write-Host "done." -ForegroundColor Green
Write-Host ""
Write-Host "Try a real inference (replace input/output paths):" -ForegroundColor White
Write-Host "  cd $repo"
Write-Host "  $pyenv inference_cli.py ``"
Write-Host "    --input  C:\path\to\input.mp4 ``"
Write-Host "    --output C:\path\to\output.mp4 ``"
Write-Host "    --resolution 1080 ``"
Write-Host "    --batch_size 5 ``"
Write-Host "    --video_backend ffmpeg"
Write-Host ""
Write-Host "Notes:" -ForegroundColor White
Write-Host "  - --batch_size must be 4n+1 (1, 5, 9, 13, 17, 21...)"
Write-Host "  - --resolution is the SHORT side of the output (1080 → 1080p)"
Write-Host "  - First inference downloads ~5 GB diffusers/peft model cache to ~/.cache/huggingface"
