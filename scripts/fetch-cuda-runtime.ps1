# Fetch CUDA / cuDNN runtime DLLs from NVIDIA's official PyPI wheels
# (no NVIDIA developer-account login needed) and stage them where the
# CUDA-feature `ort` sidecars look them up at runtime.
#
# ort 2.0.0-rc.10 ships an onnxruntime native lib built against CUDA 12.x
# + cuDNN 9.x — pin those major versions here. If you bump ort, re-check
# the EP build manifest and adjust.
#
# Output:
#   src-tauri/binaries/runtime/   ← canonical place (Tauri bundles this)
#   target/debug/                 ← so `tauri dev` finds them
#   target/release/               ← so unbundled `cargo run --release` works
#
# Re-runnable: skips already-downloaded wheels and already-staged DLLs.

param(
    [string]$DepsDir = "$PSScriptRoot\..\cuda-deps",
    [string]$Proxy = "",   # e.g. "http://127.0.0.1:10809" for a local V2Ray/Clash front
    [switch]$Force
)

$ErrorActionPreference = "Stop"
$root = Resolve-Path "$PSScriptRoot\.."

# CUDA major: must match the ort prebuilt onnxruntime EP. Bump in lockstep.
$cudaMajor = 12
$packages = @(
    "nvidia-cuda-runtime-cu$cudaMajor",
    "nvidia-cublas-cu$cudaMajor",
    "nvidia-cudnn-cu$cudaMajor",
    "nvidia-cufft-cu$cudaMajor"
)

$DepsDir = (New-Item -ItemType Directory -Path $DepsDir -Force).FullName
Write-Host "downloading CUDA $cudaMajor wheels into $DepsDir" -ForegroundColor Cyan

foreach ($pkg in $packages) {
    Write-Host ">> $pkg" -ForegroundColor Cyan
    # Don't pipe pip's output — it uses carriage returns for the progress
    # bar, which a `| ForEach-Object` would buffer until the line finally
    # ends, hiding all progress until the download finishes. Letting pip
    # write straight to the parent console preserves the live %.
    $pipArgs = @(
        "-m", "pip", "download", $pkg,
        "--dest", $DepsDir,
        "--no-deps",
        "--platform", "win_amd64",
        "--only-binary=:all:",
        "--progress-bar", "on",
        "--python-version", "3.10"
    )
    if ($Proxy) { $pipArgs += @("--proxy", $Proxy) }
    & python @pipArgs
    if ($LASTEXITCODE -ne 0) { throw "pip download failed for $pkg" }
}

# Extract wheels (they're just zips). Each puts its DLLs under nvidia/<name>/bin/.
$extractDir = Join-Path $DepsDir "_extracted"
if (Test-Path $extractDir) { Remove-Item -Recurse -Force $extractDir }
New-Item -ItemType Directory -Path $extractDir | Out-Null

Get-ChildItem $DepsDir -Filter "*.whl" | ForEach-Object {
    Write-Host "extracting $($_.Name)" -ForegroundColor DarkGray
    $target = Join-Path $extractDir $_.BaseName
    Expand-Archive -Path $_.FullName -DestinationPath $target -Force
}

# Collect every DLL under any extracted nvidia/*/bin/ folder.
$dlls = Get-ChildItem -Path $extractDir -Recurse -Filter "*.dll" -ErrorAction SilentlyContinue |
    Where-Object { $_.FullName -match '\\nvidia\\[^\\]+\\bin\\' }

if (-not $dlls -or $dlls.Count -eq 0) {
    throw "No DLLs found in extracted wheels — wheel layout may have changed"
}

# Three staging targets: bundle source + dev/release exe dirs.
$stageDirs = @(
    (Join-Path $root "src-tauri\binaries\runtime"),
    (Join-Path $root "target\debug"),
    (Join-Path $root "target\release")
) | ForEach-Object { (New-Item -ItemType Directory -Path $_ -Force).FullName }

Write-Host "staging $($dlls.Count) DLLs into:" -ForegroundColor Cyan
foreach ($d in $stageDirs) { Write-Host "  - $d" }

$totalBytes = 0
foreach ($dll in $dlls) {
    foreach ($dst in $stageDirs) {
        $out = Join-Path $dst $dll.Name
        if (-not $Force -and (Test-Path $out) -and ((Get-Item $out).Length -eq $dll.Length)) {
            continue # already up-to-date
        }
        Copy-Item $dll.FullName $out -Force
    }
    $totalBytes += $dll.Length
    Write-Host ("  {0,-40}  {1,8:N0} KB" -f $dll.Name, ($dll.Length / 1KB))
}

$totalMb = [math]::Round($totalBytes / 1MB, 1)
Write-Host "done. staged $($dlls.Count) DLLs, $totalMb MB each location." -ForegroundColor Green
Write-Host ""
Write-Host "Next: rebuild CUDA sidecars and restart tauri dev to pick them up." -ForegroundColor Yellow
