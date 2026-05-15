# One-time setup for the DOVE native VSR runner.
#
# DOVE (NeurIPS 2025, Zheng Chen et al.) — "Efficient One-Step Diffusion
# Model for Real-World Video Super-Resolution". Built on top of the
# CogVideoX1.5-5B text-to-video diffusion transformer, distilled to a
# SINGLE sampling step (vs. 30+ for SeedVR2). Promises ~28× speedup over
# multi-step diffusion VSR while keeping comparable quality on real-world
# benchmarks (UDM10, SPMCS, YouHQ40, RealVSR, MVSR4x, VideoLQ).
#
# Why we'd want this alongside SeedVR2:
#   - SeedVR2 still needs 30 sampling steps even though "one-step adversarial
#     post-training" — temporal attention + diffusion sampling is heavy.
#   - DOVE is genuinely single-step (one denoising pass), so the per-frame
#     wallclock should be dramatically lower at the cost of a much heavier
#     5B-parameter backbone (vs. SeedVR2 3B/7B).
#   - Trade-off: DOVE inherits temporal consistency from CogVideoX (a true
#     video diffusion model), so flicker should be intrinsically lower than
#     per-frame ESRGAN — but not necessarily lower than SeedVR2.
#
# Layout:
#   %APPDATA%\nsay\runners\dove\
#   ├── repo\                       (zhengchen1999/DOVE clone)
#   ├── .venv\                      (uv-managed venv, Python 3.11)
#   └── repo\pretrained_models\
#       ├── DOVE\                   (Stage-2 weights from Google Drive)
#       ├── CogVideoX1.5-5B\        (HF snapshot from THUDM)
#       └── prompt_embeddings\      (precomputed empty-prompt embedding;
#                                    may ship inside DOVE checkpoint or
#                                    needs manual fetch — see Notes below)
#
# Disk budget (worst case):
#   - CogVideoX1.5-5B HF snapshot : ~25-30 GB (transformer FP16 + VAE + T5)
#   - DOVE Stage-2 weights        : ~5-10 GB (size unknown until gdown runs)
#   - venv (torch + cuDNN)        : ~5 GB
#   - Total                       : ~35-50 GB
#
# WARNING: this is a heavyweight install. Run with -SkipCogVideoX if you
# already have CogVideoX1.5-5B downloaded somewhere else (set --model_path
# to point at it instead).
#
# Known unknowns (per README, Dec 2025 — repo is young, NeurIPS 2025):
#   1. DOVE Stage-2 may be a self-contained diffusers pipeline OR may be
#      LoRA/adapter weights requiring CogVideoX1.5-5B base. README is
#      ambiguous. We download both to be safe.
#   2. The Google Drive link points to a folder/file of unknown layout
#      (zip? folder? single safetensors?). Script tries gdown --folder
#      first, falls back to single-file download.
#   3. prompt_embeddings/ file (e3b0c4...safetensors) has no documented
#      download — likely bundled in DOVE Stage-2 checkpoint. If smoke test
#      fails on missing prompt embedding, that's the next thing to debug.

param(
    [switch]$Force,            # nuke and rebuild venv + re-clone + redownload
    [switch]$SkipWeights,      # skip ALL weight downloads
    [switch]$SkipCogVideoX,    # skip CogVideoX1.5-5B HF snapshot (~25 GB)
    [switch]$SkipDove,         # skip DOVE Stage-2 Google Drive download
    [string]$CudaIndex = "https://download.pytorch.org/whl/cu124"
)

$ErrorActionPreference = "Stop"

# Same Windows-console UTF-8 fix we needed for SeedVR2 — DOVE/diffusers
# stack will likewise emit non-ASCII at import time. Force utf-8 IO so
# RU/non-UTF8 consoles don't crash on first print.
$env:PYTHONIOENCODING = "utf-8"
$env:PYTHONUTF8 = "1"

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

# uv bootstrap (same pattern as setup-seedvr2.ps1).
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
Write-Host "    uv: $($uv.Source)" -ForegroundColor DarkGray

# --- Paths --------------------------------------------------------------

$root          = Join-Path $env:APPDATA "nsay\runners\dove"
$repo          = Join-Path $root "repo"
$venv          = Join-Path $root ".venv"
$pretrained    = Join-Path $repo "pretrained_models"
$doveDir       = Join-Path $pretrained "DOVE"
$cogDir        = Join-Path $pretrained "CogVideoX1.5-5B"
$promptDir     = Join-Path $pretrained "prompt_embeddings"
$pyenv         = Join-Path $venv "Scripts\python.exe"
$repoUrl       = "https://github.com/zhengchen1999/DOVE.git"

# DOVE Stage-2 (Final) Google Drive file ID — from README.
# https://drive.google.com/file/d/1Nl3XoJndMtpu6KPFcskUTkI0qWBiSXF2/view
$doveDriveId   = "1Nl3XoJndMtpu6KPFcskUTkI0qWBiSXF2"

Write-Host ""
Write-Host "    root           : $root" -ForegroundColor DarkGray
Write-Host "    repo           : $repo" -ForegroundColor DarkGray
Write-Host "    venv           : $venv (Python 3.11)" -ForegroundColor DarkGray
Write-Host "    pretrained     : $pretrained" -ForegroundColor DarkGray
Write-Host "      DOVE         : $doveDir" -ForegroundColor DarkGray
Write-Host "      CogVideoX    : $cogDir $(if ($SkipCogVideoX) {'(SKIPPED)'} else {'(~25 GB)'})" -ForegroundColor DarkGray
Write-Host ""

if (-not (Test-Path $root)) { New-Item -ItemType Directory -Force -Path $root | Out-Null }

# --- Step 1: clone repo ------------------------------------------------

if ($Force -and (Test-Path $repo)) {
    Write-Host "==> -Force: removing existing clone $repo" -ForegroundColor Cyan
    Remove-Item -Recurse -Force $repo
}
if (Test-Path $repo) {
    Write-Host "==> updating existing DOVE clone" -ForegroundColor Cyan
    & git -C $repo fetch --depth 1 origin main
    if ($LASTEXITCODE -ne 0) { throw "git fetch failed" }
    & git -C $repo reset --hard origin/main
    if ($LASTEXITCODE -ne 0) { throw "git reset failed" }
} else {
    Write-Host "==> git clone $repoUrl" -ForegroundColor Cyan
    & git clone --depth 1 $repoUrl $repo
    if ($LASTEXITCODE -ne 0) { throw "git clone failed" }
}

# --- Step 2: venv (Python 3.11 — DOVE requirement) --------------------

if ($Force -and (Test-Path $venv)) {
    Write-Host "==> -Force: removing existing venv $venv" -ForegroundColor Cyan
    Remove-Item -Recurse -Force $venv
}
if (-not (Test-Path $venv)) {
    Write-Host "==> creating venv at $venv (uv will manage Python 3.11)" -ForegroundColor Cyan
    # DOVE README explicitly requires Python 3.11 (conda create -n DOVE python=3.11).
    # SwissArmyTransformer + some other deps have wheel issues on 3.12+.
    & uv venv --python 3.11 $venv
    if ($LASTEXITCODE -ne 0) { throw "uv venv failed" }
}
if (-not (Test-Path $pyenv)) {
    throw "venv python missing at $pyenv (creation may have failed silently)"
}

# --- Step 3: torch first (forces CUDA index) --------------------------

Write-Host "==> installing torch + torchvision from $CudaIndex" -ForegroundColor Cyan
Write-Host "    ~2.5 GB download (cuDNN 9 bundled in wheel)" -ForegroundColor DarkGray
& uv pip install --python $pyenv --index-url $CudaIndex torch torchvision
if ($LASTEXITCODE -ne 0) { throw "torch install failed (cu124 index unreachable?)" }

# --- Step 4: DOVE deps -------------------------------------------------

Write-Host "==> installing DOVE requirements" -ForegroundColor Cyan
$reqFile = Join-Path $repo "requirements.txt"
if (-not (Test-Path $reqFile)) { throw "requirements.txt not found at $reqFile" }
& uv pip install --python $pyenv -r $reqFile
if ($LASTEXITCODE -ne 0) { throw "requirements.txt install failed" }

# Extras the README mentions explicitly outside requirements.txt:
#   - diffusers[torch]: README says `pip install diffusers["torch"]`. The
#     "torch" extra ensures torch-tied versions of diffusers utilities.
#   - pyiqa: image-quality metrics used by eval_metrics.py (skip if you
#     don't plan to evaluate, but small enough to ship).
#   - gdown: needed to fetch DOVE Stage-2 from Google Drive (no HF mirror
#     exists at time of writing).
#   - huggingface_hub: for snapshot_download of CogVideoX1.5-5B.
Write-Host "==> installing extras (diffusers[torch], pyiqa, gdown, huggingface_hub)" -ForegroundColor Cyan
& uv pip install --python $pyenv "diffusers[torch]" pyiqa gdown "huggingface_hub<1.0"
if ($LASTEXITCODE -ne 0) { throw "extras install failed" }

# Same transformers 5.x KeyError regression that hit SeedVR2 — pin to 4.x.
# DOVE was developed against transformers >=4.46.2 (per requirements.txt),
# so 4.57+ is safe.
Write-Host "==> pinning transformers<5 (5.x has flash_attn KeyError regression)" -ForegroundColor Cyan
& uv pip install --python $pyenv "transformers<5.0"
if ($LASTEXITCODE -ne 0) { throw "transformers downgrade failed" }

# --- Step 5: weights ---------------------------------------------------

if (-not $SkipWeights) {
    if (-not (Test-Path $pretrained)) { New-Item -ItemType Directory -Force -Path $pretrained | Out-Null }

    # 5a. CogVideoX1.5-5B from HuggingFace (THUDM org).
    # Skip with -SkipCogVideoX if you have it elsewhere.
    if (-not $SkipCogVideoX) {
        Write-Host ""
        Write-Host "==> downloading CogVideoX1.5-5B from HuggingFace (~25 GB)" -ForegroundColor Cyan
        Write-Host "    this is the SLOW part — go grab coffee, will take 15-60 min" -ForegroundColor DarkGray

        $dlCog = @"
from huggingface_hub import snapshot_download
import os
target = r'$cogDir'
os.makedirs(target, exist_ok=True)
print(f'Downloading THUDM/CogVideoX1.5-5B to {target}...')
# allow_patterns keeps us to the actual model files, skips example.py / README.
# Note: this is the ZAI-org / THUDM repo. If 404s, try 'zai-org/CogVideoX1.5-5B'
# (THUDM rebranded to zai-org in 2025 but the THUDM mirror still works).
snapshot_download(
    repo_id='THUDM/CogVideoX1.5-5B',
    local_dir=target,
    allow_patterns=['*.json', '*.safetensors', '*.txt', '*.model', 'tokenizer/*', 'tokenizer_2/*'],
    max_workers=4,
)
print('OK — CogVideoX1.5-5B in place')
"@
        & $pyenv -c $dlCog
        if ($LASTEXITCODE -ne 0) {
            Write-Warning "CogVideoX1.5-5B download failed. Try -SkipCogVideoX and download manually."
            Write-Warning "Manual: huggingface-cli download THUDM/CogVideoX1.5-5B --local-dir $cogDir"
            exit 1
        }
    } else {
        Write-Host "==> skipping CogVideoX1.5-5B (-SkipCogVideoX)" -ForegroundColor Yellow
        Write-Host "    point inference_script.py --model_path at your existing copy" -ForegroundColor DarkGray
    }

    # 5b. DOVE Stage-2 weights from Google Drive via gdown.
    # File ID is hardcoded from README (Dec 2025). Format is unknown until
    # download completes — could be a folder, a zip, or a single safetensors.
    # gdown handles all three; we extract zip if needed.
    if (-not $SkipDove) {
        if (-not (Test-Path $doveDir)) { New-Item -ItemType Directory -Force -Path $doveDir | Out-Null }

        Write-Host ""
        Write-Host "==> downloading DOVE Stage-2 weights from Google Drive" -ForegroundColor Cyan
        Write-Host "    file id: $doveDriveId" -ForegroundColor DarkGray
        Write-Host "    (gdown will print actual size once Google responds)" -ForegroundColor DarkGray

        $dlDove = @"
import gdown, os, sys, zipfile, shutil
target = r'$doveDir'
file_id = '$doveDriveId'

# gdown.download_folder works if the Drive link is a folder. If it's a single
# file, falls through to gdown.download.
url_folder = f'https://drive.google.com/drive/folders/{file_id}'
url_file   = f'https://drive.google.com/uc?id={file_id}'

print('Trying as folder first...')
try:
    paths = gdown.download_folder(url_folder, output=target, quiet=False, use_cookies=False)
    if paths:
        print(f'OK — downloaded {len(paths)} file(s) as folder')
        sys.exit(0)
except Exception as e:
    print(f'Folder mode failed ({e}), trying single-file mode...')

# Single-file fallback. Output filename is auto-detected by gdown from
# the Drive Content-Disposition header.
out = gdown.download(url_file, output=target + os.sep, quiet=False, fuzzy=True)
if not out:
    print('ERROR: gdown returned no path. Check Google Drive quota or download manually.')
    sys.exit(1)
print(f'OK — downloaded to {out}')

# If it's a zip, extract in place and remove the archive.
if out.endswith('.zip'):
    print(f'Extracting zip {out}...')
    with zipfile.ZipFile(out) as zf:
        zf.extractall(target)
    os.remove(out)
    print('Extracted and removed zip')
"@
        & $pyenv -c $dlDove
        if ($LASTEXITCODE -ne 0) {
            Write-Warning "DOVE Stage-2 download failed."
            Write-Warning "Likely causes: Google Drive quota hit OR file requires manual login."
            Write-Warning "Manual download: https://drive.google.com/file/d/$doveDriveId/view"
            Write-Warning "Place the result in: $doveDir"
            exit 1
        }
    } else {
        Write-Host "==> skipping DOVE Stage-2 (-SkipDove)" -ForegroundColor Yellow
    }
} else {
    Write-Host "==> skipping ALL weights (-SkipWeights)" -ForegroundColor Yellow
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
import diffusers, transformers, peft, decord
print('diffusers :', diffusers.__version__)
print('transformers:', transformers.__version__)
print('peft      :', peft.__version__)
print('decord    :', decord.__version__)
# CogVideoXPipeline is what DOVE inference_script.py loads. Check it imports.
from diffusers import CogVideoXPipeline
print('CogVideoXPipeline: OK (import only, not loaded)')
print('OK — DOVE deps importable, CUDA visible')
"@
& $pyenv -c $smoke
if ($LASTEXITCODE -ne 0) {
    Write-Warning "Smoke test failed — deps installed but something doesn't import. See above."
    exit 1
}

# Verify inference_script.py exists and parses --help.
Write-Host ""
Write-Host "==> verifying inference_script.py --help" -ForegroundColor Cyan
$cli = Join-Path $repo "inference_script.py"
if (-not (Test-Path $cli)) {
    Write-Warning "inference_script.py not found at $cli — repo layout may have changed."
    exit 1
}
Push-Location $repo
try {
    & $pyenv inference_script.py --help 2>&1 | Select-Object -First 30
    if ($LASTEXITCODE -ne 0) {
        Write-Warning "inference_script.py --help failed. See above."
        exit 1
    }
} finally {
    Pop-Location
}

# --- Done --------------------------------------------------------------

Write-Host ""
Write-Host "done." -ForegroundColor Green
Write-Host ""
Write-Host "Try a real inference (DOVE expects a DIRECTORY of videos, not a single file):" -ForegroundColor White
Write-Host "  `$env:PYTHONIOENCODING = 'utf-8'   # required on RU/non-UTF8 Windows consoles"
Write-Host "  `$env:PYTHONUTF8 = '1'"
Write-Host "  cd $repo"
Write-Host "  $pyenv inference_script.py ``"
Write-Host "    --input_dir   C:\path\to\folder_with_videos ``"
Write-Host "    --model_path  $doveDir ``"
Write-Host "    --output_path C:\path\to\output_dir ``"
Write-Host "    --is_vae_st ``"
Write-Host "    --upscale 4 ``"
Write-Host "    --save_format yuv420p"
Write-Host ""
Write-Host "Notes:" -ForegroundColor White
Write-Host "  - input is a DIRECTORY (DOVE globs *.mp4/.avi/.mov/.mkv inside)."
Write-Host "  - --is_vae_st enables VAE slicing+tiling (memory-friendly, recommended)."
Write-Host "  - default --upscale is 4. Use --upscale 1 for restoration without scale."
Write-Host "  - on 12 GB GPUs add --is_cpu_offload if you OOM (sequential CPU offload)."
Write-Host "  - --tile_size_hw H W and --chunk_len N reduce memory at speed cost."
Write-Host "  - if prompt_embeddings/...safetensors is missing at runtime, that file"
Write-Host "    needs to be fetched separately — README is silent on its source. Search"
Write-Host "    DOVE GitHub Issues for 'prompt_embeddings' if you hit this."
