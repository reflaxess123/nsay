# nsay — Local AI Image / Video Toolkit

Desktop-приложение: удаление фона, апскейл и интерполяция кадров. Локальный
inference на ONNX-моделях, без облака. Tauri 2 + Svelte 5 фронтенд, Rust
бэкенд с sidecar-бинарями per backend (CUDA / DirectML / Vulkan / CoreML / CPU).

## Архитектура

Зеркалит [flov](D:/CProjs/flov) — тот же sidecar-pattern, тот же UI-стек,
те же design-tokens. Главное отличие: вместо одного типа sidecar (whisper)
здесь три — `nsay-rembg-*`, `nsay-upscale-*`, `nsay-interp-*`. Каждый sidecar
жёстко привязан к одному backend (cuda/dml/vulkan/coreml/cpu) — фичи `ort`
не унифицируются между bin'ами благодаря `exclude = ["crates/*"]` в
workspace root.

```
nsay/
├── Cargo.toml          # workspace: members=["src-tauri"], exclude=["crates/*"]
├── .cargo/config.toml  # CUDA/MSVC env (unchanged from flov)
├── nsay.toml           # дев-конфиг (gitignored)
├── dev.cmd             # tauri dev
├── src-tauri/          # main app (nsay_app.exe)
│   └── src/
│       ├── lib.rs            # Tauri Builder + tools registry
│       ├── config.rs         # nsay.toml read + surgical write (toml_edit)
│       ├── tools/
│       │   ├── mod.rs        # sidecar resolve + spawn helpers
│       │   ├── rembg.rs      # background removal job runner
│       │   ├── upscale.rs    # super-resolution job runner
│       │   └── interp.rs     # frame interpolation job runner
│       ├── models.rs         # ONNX catalogue (HF download URLs, sha256, sizes)
│       ├── models_cmd.rs     # Tauri commands: list/download/delete model
│       ├── state_cmd.rs      # backend selector commands
│       └── ffmpeg.rs         # video decode/encode (subprocess to bundled ffmpeg)
├── crates/             # sidecar binaries (workspace excluded)
│   ├── nsay-rembg-cpu/      # ort 2.x, CPU EP
│   ├── nsay-rembg-cuda/     # ort + cuda feature
│   ├── nsay-rembg-dml/      # ort + directml feature
│   ├── nsay-rembg-coreml/   # ort + coreml feature (macOS)
│   ├── nsay-rembg-vulkan/   # burn + burn-wgpu (post-MVP)
│   ├── nsay-upscale-*/      # same fan-out, Real-ESRGAN
│   └── nsay-interp-*/       # same fan-out, RIFE / FILM
├── ui/                 # SvelteKit (adapter-static, port 1420)
│   └── src/routes/
│       ├── +layout.svelte   # tab nav (Rembg / Upscale / Interp / Settings)
│       ├── rembg/+page.svelte
│       ├── upscale/+page.svelte
│       ├── interp/+page.svelte
│       └── settings/+page.svelte
└── scripts/
    ├── build-sidecars.ps1   # копия паттерна из flov
    └── download-models.ps1  # ручной prefetch ONNX из HF (UI делает то же)
```

## Wire-протокол sidecar

Все sidecars одного семейства (rembg/upscale/interp) разделяют протокол.
Источник истины: `crates/nsay-rembg-cpu/src/main.rs`.

```
args:   --model <onnx_path> --input <path> --output <path> [--param key=value ...]
stderr: human-readable progress (one line: "stage=X pct=NN") и ошибки
stdout: пусто или JSON метаданные (e.g. {"width":1920,"height":1080})
exit:   0 success, 1 failure
```

Файл-based намеренно (а не pipe stdin/stdout как у flov whisper) — для
картинок и кадров проще: меньше копирования через пайпы, фронт может
показать input/output превью прямо из tmp-файлов.

## Backend selection

`tools::resolve_sidecar(tool, choice)`:
1. `NSAY_BACKEND` env var (debug override)
2. `[backend].choice` из nsay.toml (тогглится из Settings → Backend)
3. `auto` → priority `[cuda, dml, vulkan, coreml, cpu]`, первый существующий
   рядом с exe бинарь `nsay-<tool>-<backend>(.exe)`

Сменa backend'а из Settings — без рестарта (resolve на каждый job).

## Модели

Каталог в `src-tauri/src/models.rs` — id → {url, sha256, size, file_name,
families: ["rembg"|"upscale"|"interp"]}. UI скачивает в `models/` рядом с exe.

Текущий стартовый набор:
- **rembg**: `bria-rmbg-1.4-fp16` (88 MB) — `briaai/RMBG-1.4/onnx/model_fp16.onnx`
- **upscale**: `real-esrgan-x4plus` (67 MB) — `qualcomm/Real-ESRGAN-x4plus`
- **interp**: `rife-4.22` (TBD — экспортируется отдельно)

## Сборка

Main app:
```powershell
.\dev.cmd                                       # dev с hot reload
.\ui\node_modules\.bin\tauri.cmd build          # release
```

Sidecars (отдельная команда — workspace excluded):
```powershell
.\scripts\build-sidecars.ps1                    # все доступные
.\scripts\build-sidecars.ps1 -Tool rembg        # фильтр по tool
.\scripts\build-sidecars.ps1 -Backend cuda      # фильтр по backend
```

## CUDA / DirectML / Vulkan deps

- **CUDA**: бинарь линкуется с `ort` (cuda feature). На юзера нужен NVIDIA
  driver. Сборка тащит cuDNN/cuBLAS — `build-sidecars.ps1` стейджит DLLs
  рядом с sidecar (как flov делает с cublas).
- **DirectML**: `ort` (directml feature) + `DirectML.dll` (~10 MB,
  стейджится скриптом). Работает на любой DirectX 12 GPU (AMD/Intel/NVIDIA)
  на Windows 10+. Дефолт на Win.
- **Vulkan**: `burn` + `burn-wgpu`. Требует Vulkan loader (есть на всех
  современных GPU). Без CUDA / DirectML deps.
- **CoreML**: `ort` (coreml feature). Apple Silicon — ANE + GPU.
- **CPU**: `ort` (default features). Везде fallback.
