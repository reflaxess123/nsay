# Downloads a static Windows ffmpeg + ffprobe build with NVENC/QSV/AMF
# encoders enabled (BtbN GPL build) and stages the two binaries into
# src-tauri/binaries/ffmpeg/.
#
# Usage: .\scripts\fetch-ffmpeg.ps1 [-Proxy 127.0.0.1:10809]
#
# Why BtbN GPL build (not gyan.dev essentials):
#   - includes h264_nvenc, hevc_nvenc, h264_qsv, hevc_qsv, h264_amf, hevc_amf
#   - statically linked, no extra DLL dependencies
#   - tracking master so it has up-to-date filter graph features

param(
    [string]$Proxy = ""
)

$ErrorActionPreference = "Stop"
$root = Resolve-Path "$PSScriptRoot\.."
$dest = Join-Path $root "src-tauri\binaries\ffmpeg"
$tmp  = Join-Path $env:TEMP "nsay-ffmpeg-$(Get-Random)"
New-Item -ItemType Directory -Force -Path $dest | Out-Null
New-Item -ItemType Directory -Force -Path $tmp  | Out-Null

$url = "https://github.com/BtbN/FFmpeg-Builds/releases/download/latest/ffmpeg-master-latest-win64-gpl.zip"
$zip = Join-Path $tmp "ffmpeg.zip"

Write-Host ">> downloading ffmpeg from BtbN" -ForegroundColor Cyan
$wcArgs = @{ Uri = $url; OutFile = $zip; UseBasicParsing = $true }
if ($Proxy) { $wcArgs.Proxy = "http://$Proxy" }
Invoke-WebRequest @wcArgs

Write-Host ">> extracting" -ForegroundColor Cyan
Expand-Archive -Path $zip -DestinationPath $tmp -Force

# BtbN archive structure: ffmpeg-master-latest-win64-gpl/bin/{ffmpeg,ffprobe,ffplay}.exe
$binDir = Get-ChildItem -Path $tmp -Directory | Where-Object { $_.Name -like "ffmpeg-*" } | Select-Object -First 1
if (-not $binDir) { throw "could not locate extracted ffmpeg root in $tmp" }

foreach ($exe in @("ffmpeg.exe", "ffprobe.exe")) {
    $src = Join-Path $binDir.FullName "bin\$exe"
    if (-not (Test-Path $src)) { throw "missing $exe in archive" }
    Copy-Item $src (Join-Path $dest $exe) -Force
}

Remove-Item -Recurse -Force $tmp

$sizes = Get-ChildItem $dest -File | ForEach-Object { "{0,-14} {1,8:N1} MB" -f $_.Name, ($_.Length / 1MB) }
Write-Host "done. staged into $dest" -ForegroundColor Green
$sizes | ForEach-Object { Write-Host "  $_" }
