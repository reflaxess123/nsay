# Manual model prefetch from Hugging Face.
# UI does the same via Settings → Models, but this is handy for CI / dev
# scripting / "just give me everything" flows.
#
# Usage:
#   .\scripts\download-models.ps1                    # all
#   .\scripts\download-models.ps1 -Id bria-rmbg-1.4-fp16

param(
    [string]$Id = "all"
)

$ErrorActionPreference = "Stop"
$root = Resolve-Path "$PSScriptRoot\.."
$dest = Join-Path $root "models"
if (-not (Test-Path $dest)) { New-Item -ItemType Directory -Path $dest | Out-Null }

# Mirror src-tauri/src/models.rs catalog. Keep in sync.
$catalog = @{
    "bria-rmbg-1.4" = @{
        url      = "https://huggingface.co/briaai/RMBG-1.4/resolve/main/onnx/model.onnx"
        filename = "bria-rmbg-1.4.onnx"
    }
    "bria-rmbg-1.4-fp16" = @{
        url      = "https://huggingface.co/briaai/RMBG-1.4/resolve/main/onnx/model_fp16.onnx"
        filename = "bria-rmbg-1.4-fp16.onnx"
    }
    "ben2-base" = @{
        url      = "https://huggingface.co/PramaLLC/BEN2/resolve/main/BEN2_Base.onnx"
        filename = "ben2-base.onnx"
    }
    "real-esrgan-x4" = @{
        url      = "https://huggingface.co/crj/dl-ws/resolve/main/real_esrgan_x4.onnx"
        filename = "real-esrgan-x4.onnx"
    }
    "real-esrgan-x2" = @{
        url      = "https://huggingface.co/crj/dl-ws/resolve/main/real_esrgan_x2.onnx"
        filename = "real-esrgan-x2.onnx"
    }
    "real-hatgan-x4" = @{
        url      = "https://huggingface.co/crj/dl-ws/resolve/main/real_hatgan_x4.onnx"
        filename = "real-hatgan-x4.onnx"
    }
    "rife-4.9" = @{
        url      = "https://huggingface.co/yuvraj108c/rife-onnx/resolve/main/rife49_ensemble_True_scale_1_sim.onnx"
        filename = "rife-4.9.onnx"
    }
}

function Download-One($key) {
    if (-not $catalog.ContainsKey($key)) {
        Write-Warning "unknown model id: $key"
        return
    }
    $entry = $catalog[$key]
    $out = Join-Path $dest $entry.filename
    if (Test-Path $out) {
        Write-Host "skip: $($entry.filename) exists" -ForegroundColor Yellow
        return
    }
    Write-Host ">> downloading $($entry.filename) from $($entry.url)" -ForegroundColor Cyan
    Invoke-WebRequest -Uri $entry.url -OutFile $out -UseBasicParsing
}

if ($Id -eq "all") {
    foreach ($k in $catalog.Keys) { Download-One $k }
} else {
    Download-One $Id
}

Write-Host "done." -ForegroundColor Green
