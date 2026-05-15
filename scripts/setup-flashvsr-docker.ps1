# One-time setup for the FlashVSR-Pro Docker backend (`-Backend docker`).
#
# Maps the README install path from LujiaJin/FlashVSR-Pro into something
# nsay can drive headlessly:
#   1. Clone FlashVSR-Pro --recursive (pulls in the Block-Sparse-Attention
#      submodule that the Dockerfile compiles in-image).
#   2. Build flashvsr-pro:latest (~30 min on first run, then cached).
#   3. Download the FlashVSR-v1.1 ONNX/torch weights from HuggingFace into
#      %APPDATA%\nsay\models\flashvsr-v1.1, which nsay-vidsr-docker.exe
#      bind-mounts read-only into the container at /workspace/FlashVSR-Pro/
#      models/FlashVSR-v1.1. Weights are kept on the host so re-pulling
#      the image doesn't re-download multi-GB checkpoints.
#
# Why all of this lives outside the container image:
# - Image stays modest (still ~10-15 GB after CUDA + PyTorch + deps), but
#   weights add another ~6 GB which would make `docker pull` painful if we
#   ever published the image.
# - Updating FlashVSR weights = `git lfs pull` in the host folder, no
#   image rebuild.
#
# Prerequisites the user must have done themselves:
# - Install Docker Desktop for Windows (>= 4.27, Sep 2025).
# - Enable WSL2 backend AND "Use the WSL2 based engine" in Settings.
# - Settings → Resources → WSL Integration → enable for Ubuntu (or
#   whichever distro Docker uses).
# - NVIDIA driver >= 565 on the host. The NVIDIA Container Toolkit ships
#   inside Docker Desktop's WSL distro nowadays — no separate install.
# - Verify GPU passthrough: `docker run --rm --gpus all nvidia/cuda:12.4.0-base-ubuntu22.04 nvidia-smi`
#   should print your GPU. If not, fix that before running this script.
#
# Usage:
#   .\scripts\setup-flashvsr-docker.ps1               # full setup
#   .\scripts\setup-flashvsr-docker.ps1 -SkipBuild    # only refresh weights
#   .\scripts\setup-flashvsr-docker.ps1 -SkipWeights  # only rebuild image
#   .\scripts\setup-flashvsr-docker.ps1 -Force        # re-clone & rebuild from scratch

param(
    [switch]$SkipBuild,
    [switch]$SkipWeights,
    [switch]$Force,
    [string]$ImageTag = "flashvsr-pro:latest",
    [string]$Repo = "https://github.com/LujiaJin/FlashVSR-Pro.git",
    [string]$WeightsRepo = "https://huggingface.co/JunhaoZhuang/FlashVSR-v1.1"
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

Need-Cmd "docker" "Install Docker Desktop for Windows (https://www.docker.com/products/docker-desktop)." | Out-Null
Need-Cmd "git"    "Install Git for Windows (https://git-scm.com/download/win)." | Out-Null

# git-lfs is needed for HuggingFace weight download. If absent, fall back
# to a docker-based pull below, but warn — the docker fallback is slow.
$gitLfs = Get-Command "git-lfs" -ErrorAction SilentlyContinue

# Docker daemon must be reachable, otherwise every later command hangs.
try {
    $null = & docker info 2>&1
    if ($LASTEXITCODE -ne 0) { throw "docker info failed (exit $LASTEXITCODE)" }
} catch {
    Write-Error "Docker daemon is not reachable. Start Docker Desktop and wait for it to finish initializing, then retry."
    exit 1
}

# Quick GPU passthrough probe so we fail in 5 seconds instead of 25 minutes
# into a build that nothing on the host can ever execute.
Write-Host "    probing --gpus all (nvidia-smi inside cuda:12.4 base image)..." -ForegroundColor DarkGray
$gpuProbe = & docker run --rm --gpus all nvidia/cuda:12.4.0-base-ubuntu22.04 nvidia-smi 2>&1
if ($LASTEXITCODE -ne 0) {
    Write-Warning "GPU passthrough probe failed:"
    Write-Warning "  $($gpuProbe -join "`n  ")"
    Write-Warning "FlashVSR-Pro will not run without --gpus all. Fix Docker Desktop's NVIDIA integration first."
    if (-not $Force) {
        Write-Host "Aborting. Re-run with -Force to ignore." -ForegroundColor Red
        exit 1
    }
} else {
    Write-Host "    OK — Docker can see the GPU" -ForegroundColor Green
}

# Free space sanity: image build needs ~25 GB working set.
$workDrive = (Get-Location).Drive
if ($workDrive) {
    $freeGb = [math]::Round($workDrive.Free / 1GB, 1)
    if ($freeGb -lt 30) {
        Write-Warning "Only $freeGb GB free on $($workDrive.Name): — FlashVSR-Pro image build typically needs 25-30 GB."
    }
}

# --- Paths --------------------------------------------------------------

$root      = Resolve-Path "$PSScriptRoot\.."
$cloneDir  = Join-Path $env:TEMP "nsay-flashvsr-pro"
$weightDir = Join-Path $env:APPDATA "nsay\models\flashvsr-v1.1"

Write-Host ""
Write-Host "    repo clone : $cloneDir" -ForegroundColor DarkGray
Write-Host "    weights    : $weightDir" -ForegroundColor DarkGray
Write-Host "    image tag  : $ImageTag" -ForegroundColor DarkGray
Write-Host ""

# --- Step 1: clone repo (Block-Sparse-Attention is a submodule) ---------

if (-not $SkipBuild) {
    if ($Force -and (Test-Path $cloneDir)) {
        Write-Host "==> -Force: removing existing clone $cloneDir" -ForegroundColor Cyan
        Remove-Item -Recurse -Force $cloneDir
    }
    if (Test-Path $cloneDir) {
        Write-Host "==> updating existing clone $cloneDir" -ForegroundColor Cyan
        & git -C $cloneDir fetch --depth 1 origin main
        if ($LASTEXITCODE -ne 0) { throw "git fetch failed" }
        & git -C $cloneDir reset --hard origin/main
        if ($LASTEXITCODE -ne 0) { throw "git reset failed" }
        & git -C $cloneDir submodule update --init --recursive --depth 1
        if ($LASTEXITCODE -ne 0) { throw "git submodule update failed" }
    } else {
        Write-Host "==> cloning $Repo (with submodules)" -ForegroundColor Cyan
        & git clone --recursive --depth 1 $Repo $cloneDir
        if ($LASTEXITCODE -ne 0) { throw "git clone failed" }
    }

    # --- Step 1.5: patch Dockerfile in-place ---
    #
    # Upstream Dockerfile lets ninja default to N_cores parallel nvcc jobs and
    # compiles Block-Sparse-Attention for every CUDA arch in TORCH_CUDA_ARCH_LIST.
    # On a 12-core box that peaks 24+ GB RAM and trips OOM-kill on BuildKit
    # (gRPC EOF), and the multi-arch compile adds 5-10 min of waste for users
    # who only care about their own GPU. Patch in:
    #   ENV MAX_JOBS=4              — cap concurrent nvcc to 4 (~12 GB peak)
    #   ENV TORCH_CUDA_ARCH_LIST    — only the user's GPU compute capability
    # Idempotent: re-running this script doesn't double-patch.
    $arch = $env:NSAY_CUDA_ARCH
    if (-not $arch) { $arch = "8.6" }   # RTX 30xx default; override via env for other GPUs
    $maxJobs = $env:NSAY_MAX_JOBS
    if (-not $maxJobs) { $maxJobs = "4" }

    $dockerfilePath = Join-Path $cloneDir "Dockerfile"
    if (Test-Path $dockerfilePath) {
        $content = Get-Content $dockerfilePath -Raw
        if ($content -match "ENV MAX_JOBS") {
            Write-Host "==> Dockerfile already patched (MAX_JOBS / TORCH_CUDA_ARCH_LIST)" -ForegroundColor DarkGray
        } else {
            $needle = "WORKDIR /workspace/FlashVSR-Pro/Block-Sparse-Attention"
            if ($content -match [regex]::Escape($needle)) {
                Write-Host "==> patching Dockerfile: MAX_JOBS=$maxJobs, TORCH_CUDA_ARCH_LIST=`"$arch`"" -ForegroundColor Cyan
                $patch = "ENV MAX_JOBS=$maxJobs`nENV TORCH_CUDA_ARCH_LIST=`"$arch`"`n$needle"
                $content = $content -replace [regex]::Escape($needle), $patch
                Set-Content -NoNewline -Path $dockerfilePath -Value $content
            } else {
                Write-Warning "Block-Sparse-Attention WORKDIR not found in Dockerfile — skipping patch."
                Write-Warning "If build OOMs, manually add MAX_JOBS / TORCH_CUDA_ARCH_LIST envs."
            }
        }
    }

    # --- Step 2: docker build (this is the long step; CUDA kernel compile
    #     of Block-Sparse-Attention happens inside the image build) ----
    Write-Host ""
    Write-Host "==> docker build -t $ImageTag $cloneDir" -ForegroundColor Cyan
    Write-Host "    This compiles Block-Sparse-Attention CUDA kernels inside the" -ForegroundColor DarkGray
    Write-Host "    container — typically 20-30 min on first run, then cached." -ForegroundColor DarkGray
    & docker build -t $ImageTag $cloneDir
    if ($LASTEXITCODE -ne 0) {
        Write-Host ""
        Write-Host "Hint: if you see 'gRPC EOF' or BuildKit died mid-step 14," -ForegroundColor Yellow
        Write-Host "raise WSL2 memory in %USERPROFILE%\.wslconfig:" -ForegroundColor Yellow
        Write-Host "    [wsl2]" -ForegroundColor Yellow
        Write-Host "    memory=24GB" -ForegroundColor Yellow
        Write-Host "    swap=8GB" -ForegroundColor Yellow
        Write-Host "Then run: wsl --shutdown   (Docker Desktop will restart)" -ForegroundColor Yellow
        Write-Host "And re-run this script — cached steps will be re-used." -ForegroundColor Yellow
        throw "docker build failed"
    }
} else {
    Write-Host "==> skipping clone + build (-SkipBuild)" -ForegroundColor Yellow
}

# --- Step 3: weights ----------------------------------------------------

if (-not $SkipWeights) {
    if ($Force -and (Test-Path $weightDir)) {
        Write-Host "==> -Force: removing existing weights $weightDir" -ForegroundColor Cyan
        Remove-Item -Recurse -Force $weightDir
    }
    $weightParent = Split-Path -Parent $weightDir
    if (-not (Test-Path $weightParent)) {
        New-Item -ItemType Directory -Path $weightParent | Out-Null
    }

    if (Test-Path $weightDir) {
        Write-Host "==> updating existing weights $weightDir" -ForegroundColor Cyan
        if ($gitLfs) {
            & git -C $weightDir lfs pull
            if ($LASTEXITCODE -ne 0) { throw "git lfs pull failed" }
        } else {
            Write-Warning "git-lfs not installed — skipping weight refresh. Install git-lfs to update weights."
        }
    } else {
        if ($gitLfs) {
            Write-Host "==> git lfs clone $WeightsRepo" -ForegroundColor Cyan
            & git lfs clone $WeightsRepo $weightDir
            if ($LASTEXITCODE -ne 0) { throw "git lfs clone failed" }
        } else {
            Write-Warning "git-lfs not installed — using docker fallback to pull weights (slower)."
            Write-Host "==> docker pull weights via huggingface_hub" -ForegroundColor Cyan
            & docker run --rm `
                -v "${weightParent}:/host" `
                python:3.11-slim `
                bash -c "pip install --quiet huggingface_hub && python -c 'from huggingface_hub import snapshot_download; snapshot_download(repo_id=\"JunhaoZhuang/FlashVSR-v1.1\", local_dir=\"/host/flashvsr-v1.1\", local_dir_use_symlinks=False)'"
            if ($LASTEXITCODE -ne 0) { throw "docker fallback weight download failed" }
        }
    }
} else {
    Write-Host "==> skipping weights (-SkipWeights)" -ForegroundColor Yellow
}

# --- Done ---------------------------------------------------------------

Write-Host ""
Write-Host "done." -ForegroundColor Green
Write-Host ""
Write-Host "Verify:" -ForegroundColor White
Write-Host "  docker images $ImageTag        # should print one entry"
Write-Host "  ls $weightDir                   # should contain Wan2.1_VAE.pth, TCDecoder.ckpt, etc."
Write-Host ""
Write-Host "Next:" -ForegroundColor White
Write-Host "  .\scripts\build-sidecars.ps1 -Tool vidsr -Backend docker"
Write-Host "  .\dev.cmd                       # then SidePanel → Backend → Docker → run vid SR"
