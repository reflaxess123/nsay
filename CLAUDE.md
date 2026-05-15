# nsay — Local AI Image / Video Toolkit

Desktop-приложение: удаление фона, апскейл изображений и видео, интерполяция
кадров (slow-mo / boost FPS). Локальный inference на ONNX-моделях, без
облака. Tauri 2 + Svelte 5 фронтенд, Rust бэкенд с sidecar-бинарями
per backend (CUDA / DirectML / CPU; Vulkan / CoreML — post-MVP).

## Архитектура

Зеркалит [flov](D:/CProjs/flov) — тот же sidecar-pattern, тот же UI-стек,
те же design-tokens. Главное отличие: вместо одного типа sidecar (whisper)
здесь три семейства — `nsay-rembg-*`, `nsay-upscale-*`, `nsay-interp-*`.
Каждое семейство = одна shared `*-lib` крейт + по одному thin-bin крейту
на backend (cpu/cuda/dml). EP-фичи `ort` не унифицируются между bin'ами
благодаря `exclude = ["crates/*"]` в workspace root.

```
nsay/
├── Cargo.toml          # workspace: members=["src-tauri"], exclude=["crates/*"]
├── .cargo/config.toml  # CUDA/MSVC env (unchanged from flov)
├── nsay.toml           # дев-конфиг (gitignored)
├── dev.cmd             # tauri dev
├── PLAN.md             # roadmap по моделям (F0-F5) — итоги codex_audit.md
├── codex_audit.md      # аудит open-source моделей для rembg/upscale/VSR
├── src-tauri/          # main app (nsay_app.exe)
│   ├── binaries/
│   │   ├── runtime/   # CUDA/cuDNN DLLs (gitignored, fetch-cuda-runtime.ps1)
│   │   └── ffmpeg/    # ffmpeg.exe + ffprobe.exe (gitignored, fetch-ffmpeg.ps1)
│   └── src/
│       ├── lib.rs            # Tauri Builder + tools/state/models commands
│       ├── config.rs         # nsay.toml read + surgical write (toml_edit)
│       ├── ffmpeg.rs         # ffmpeg/ffprobe path + encoder auto-detection
│       ├── tools/
│       │   ├── mod.rs        # sidecar resolve + spawn helpers
│       │   ├── rembg.rs      # background removal Tauri runner
│       │   ├── upscale.rs    # image super-resolution Tauri runner
│       │   └── video.rs      # video pipeline (vid_upscale_run + vid_interp_run)
│       ├── models.rs         # ONNX catalogue (HF download URLs, sha256, sizes)
│       ├── models_cmd.rs     # Tauri commands: list/download/delete model
│       └── state_cmd.rs      # backend selector commands
├── crates/             # sidecar binaries (workspace excluded; standalone)
│   ├── nsay-rembg-lib/      # shared pipeline: BRIA-RMBG preprocessing/inference
│   ├── nsay-rembg-cpu/      # thin bin: identity provider closure
│   ├── nsay-rembg-cuda/     # thin bin: CUDAExecutionProvider
│   ├── nsay-rembg-dml/      # thin bin: DirectMLExecutionProvider
│   ├── nsay-upscale-lib/    # shared: Real-ESRGAN + tile blending + streaming
│   ├── nsay-upscale-{cpu,cuda,dml}/  # thin bins (~13 LOC each)
│   ├── nsay-interp-lib/     # shared: RIFE 4.9 streaming pipeline
│   ├── nsay-interp-{cpu,cuda,dml}/   # thin bins
│   ├── nsay-vidsr-lib/      # shared: chunked VSR via tch-rs / libtorch
│   ├── nsay-vidsr-{cpu,cuda}/        # libtorch backends (RealBasicVSR .pt)
│   └── nsay-vidsr-docker/   # docker shim → flashvsr-pro:latest container
├── ui/                 # SvelteKit (adapter-static, port 1420)
│   └── src/
│       ├── lib/
│       │   ├── components/SidePanel.svelte  # Backend + Models picker (right pane)
│       │   └── state/                       # one Svelte 5 store per page:
│       │       ├── rembg.svelte.ts          # img rembg queue
│       │       ├── upscale.svelte.ts        # img upscale queue
│       │       ├── vidUpscale.svelte.ts     # video upscale (single-file)
│       │       └── vidSlow.svelte.ts        # interp / slow-mo (single-file)
│       └── routes/
│           ├── +layout.svelte    # tab nav (4 tabs, no settings page)
│           ├── rembg/+page.svelte
│           ├── upscale/+page.svelte         # IMAGE upscale
│           ├── vid-upscale/+page.svelte     # VIDEO upscale (fast per-frame)
│           └── interp/+page.svelte          # slow-mo / boost FPS (RIFE)
└── scripts/
    ├── build-sidecars.ps1         # builds all bin crates, stages DLLs
    ├── download-models.ps1        # manual ONNX prefetch (UI does the same)
    ├── fetch-cuda-runtime.ps1     # NVIDIA cuDNN/cuBLAS/cuRT redistributables
    ├── fetch-ffmpeg.ps1           # BtbN GPL static ffmpeg+ffprobe (~400 MB)
    ├── fetch-libtorch.ps1         # libtorch 2.6.0+cu124 for tch-rs vidsr (~2.5 GB)
    ├── convert-realbasicvsr.py    # .pth → .pt TorchScript trace for vidsr-cuda/cpu
    └── setup-flashvsr-docker.ps1  # git clone --recursive + docker build + HF weights
```

## Sidecar lib refactor

Каждое семейство имеет одну `nsay-<tool>-lib` крейт — туда вынесена вся
бизнес-логика (parse_args, preprocessing, session.run, postprocessing).
Bin крейты по 13 LOC: парсят свой `--<EP>` и зовут `lib::run(closure)`.

```rust
// nsay-rembg-cuda/src/main.rs
use nsay_rembg_lib::ort::execution_providers::CUDAExecutionProvider;
fn main() {
    if let Err(e) = nsay_rembg_lib::run(|b| {
        b.with_execution_providers([CUDAExecutionProvider::default().build().error_on_failure()])
    }) {
        eprintln!("nsay-rembg-cuda error: {e:#}");
        std::process::exit(1);
    }
}
```

Feature-unification работает через cargo path-deps: lib декларирует ort
без EP features, bin добавляет `cuda` / `directml` feature, cargo сливает
их в одну ort версию с union features при сборке bin'а. Добавление нового
preset (BEN2, BiRefNet) = touch только lib.

## Wire-протокол sidecar

### Image (rembg / image upscale): file mode
```
args:   --model <onnx_path> --input <path> --output <path> [--choke F | --scale F]
stderr: progress lines `stage=<name> pct=<0..100>`, errors human-readable
stdout: пусто
exit:   0 success, 1 failure
```

### Video (upscale / interp): streaming mode
Используется video runner'ом для frame-by-frame inference в составе
3-process pipeline (см. ниже).
```
args:   --model <path> --stream --width W --height H [--scale F | --factor K]
stdin:  raw RGB frames, W*H*3 bytes per frame
stdout: raw RGB frames; upscale = same N×scale, interp = 1 + (N-1)*K
stderr: `stream-ready` once, then `frame N` per output frame
exit:   0 success, 1 failure
```

## Docker backend (FlashVSR-Pro VSR)

`vidsr` имеет 2-й runner — **diffusion-based** через Docker Desktop. Поднят
потому что современные diffusion VSR модели (FlashVSR / DOVE / DLoRAL) живут
только в PyTorch + custom CUDA kernels (Block-Sparse-Attention) которые
**не собираются на Windows native** (Block-Sparse-Attention README: "Linux.").

Архитектура:
```
nsay_app.exe → nsay-vidsr-docker.exe (Rust shim) → docker run flashvsr-pro:latest
                                                    └ python infer.py -i /in/x.mp4 -o /out/
```

Shim делает 3 вещи: (1) проверяет наличие docker.exe + image + weights dir;
(2) маппит host paths в `/in /out /workspace/.../models/FlashVSR-v1.1` через
`docker run -v`; (3) парсит tqdm в stderr контейнера и эмитит `frame N`
строки наружу — Tauri spawn_progress подхватывает в тот же UI прогресс-бар
что и libtorch sidecars.

Docker selected как явный backend (не входит в `BACKEND_PRIORITY`, "auto" его
не выбирает — Docker Desktop тяжёлая зависимость для пользователя). В
SidePanel docker row видим только когда `family="vidsr"`.

User onboarding:
1. Поставить Docker Desktop + WSL2 + NVIDIA Container Toolkit (внутри Docker
   Desktop в современных версиях). Проверка: `docker run --rm --gpus all
   nvidia/cuda:12.4.0-base-ubuntu22.04 nvidia-smi` должен напечатать GPU.
2. `.\scripts\setup-flashvsr-docker.ps1` — клонирует FlashVSR-Pro (recursive),
   `docker build -t flashvsr-pro:latest .` (~30 мин compile Block-Sparse-Attention
   внутри контейнера), скачивает FlashVSR-v1.1 weights с HF в `%APPDATA%/nsay/
   models/flashvsr-v1.1/` (~6 GB).
3. `.\scripts\build-sidecars.ps1 -Tool vidsr -Backend docker` — собирает шим
   (~50 KB exe, anyhow + regex only).
4. UI → vid SR → SidePanel → Backend → Docker → запуск.

UI knobs (только для docker, libtorch игнорирует):
- `mode`: `tiny` (8GB VRAM, default) / `full` (12+GB, лучше детали)
- `tile-vae` / `tile-dit`: streaming блоками — обязательно для 12GB GPU
- `audio`: keep audio track from source

## Backend selection

`tools::resolve_sidecar(tool, choice)`:
1. `NSAY_BACKEND` env var (debug override)
2. `[backend].choice` из nsay.toml (тогглится из right SidePanel)
3. `auto` → priority `[cuda, dml, vulkan, coreml, cpu]`, первый существующий
   рядом с exe бинарь `nsay-<tool>-<backend>(.exe)`

Сменa backend'а из SidePanel — без рестарта (resolve на каждый job).
Backends, для которых нет бинаря рядом с exe, в SidePanel показываются
disabled (видно архитектуру, но не кликается).

## Video pipeline

`src-tauri/src/tools/video.rs` собирает 3 процесса в один OS-pipe:

```
ffmpeg(decode) ─stdout→stdin─ sidecar(stream) ─stdout→stdin─ ffmpeg(encode + audio mux)
```

`Stdio::from(ChildStdout)` на Windows даёт HANDLE который OS пробрасывает
напрямую — без per-byte копирования через Rust. Encoder выбирается через
`ffmpeg::detect_encoder()` парсингом `ffmpeg -encoders`: NVENC > QSV > AMF
> libx264. Прогресс — через `spawn_progress` thread, который парсит
sidecar stderr `frame N` и эмитит `vid-progress` Tauri event.

**ВАЖНО**: текущий vid-upscale = per-frame image SR. Это не temporal video
SR — на видео возможны flicker/shimmering. Настоящий VSR (RealBasicVSR /
BasicVSR++) запланирован в PLAN.md F4 как отдельный tool `vidsr` на
libtorch (tch-rs), потому что ONNX export VSR моделей не работает —
mmcv:grid_sampler + deform_conv2d не registered в ORT.

Slow-mo / Boost FPS режимы (`/interp` страница):
- **boost** (default): same duration, fps × N, audio copy
- **slow**: duration × N, fps unchanged, audio dropped (`-an`)

## Модели

Каталог в `src-tauri/src/models.rs` — id → {url, sha256, size, file_name,
family, output_scale}. UI скачивает в `%APPDATA%/nsay/models/` (или путь
из `[models].dir` в nsay.toml).

Текущий набор (sync с `scripts/download-models.ps1`):
- **rembg**: `bria-rmbg-1.4` (176 MB fp32), `bria-rmbg-1.4-fp16` (88 MB)
- **upscale**: `real-esrgan-x4` (67 MB), `real-esrgan-x2` (67 MB),
  `real-hatgan-x4` (153 MB)
- **interp**: `rife-4.9` (21 MB) — yuvraj108c/rife-onnx ensemble

Roadmap по добавлению моделей (BEN2 / BiRefNet / Real-CUGAN / SwinIR /
RealBasicVSR) — в `PLAN.md`.

## Сборка

Main app:
```powershell
.\dev.cmd                                       # dev с hot reload
.\ui\node_modules\.bin\tauri.cmd build          # release
```

Sidecars (отдельная команда — workspace excluded):
```powershell
.\scripts\build-sidecars.ps1                            # все доступные
.\scripts\build-sidecars.ps1 -Tool rembg                # фильтр по tool
.\scripts\build-sidecars.ps1 -Backend cpu               # фильтр по backend
.\scripts\build-sidecars.ps1 -Backend cpu -Profile debug  # быстрая проверка
```

Скрипт пропускает `*-lib` крейты (они lib-only, не строятся как exe) и
автоматически stage'ит ort companion DLLs (`onnxruntime_providers_*`,
`DirectML.dll`) из ort cache рядом с собранным бинарём.

## Нативные зависимости

- **CUDA**: бинарь линкуется с `ort` (cuda feature). На юзера нужен NVIDIA
  driver. ort 2.0.0-rc.10 ожидает **CUDA 12.x + cuDNN 9.x runtime DLLs**.
  `.\scripts\fetch-cuda-runtime.ps1` качает официальные redistributable
  wheels с NVIDIA PyPI (no developer account), распаковывает и стейджит
  во все три места (`binaries/runtime/`, `target/debug/`, `target/release/`).
  Без cuDNN ort CUDA EP падает с явной ошибкой (после `.error_on_failure()`).

- **DirectML**: `ort` (directml feature) + `DirectML.dll` (~10 MB). Работает
  на любой DirectX 12 GPU (AMD/Intel/NVIDIA) на Windows 10+. Никаких NVIDIA
  deps. Дефолт на Win если CUDA не настроена.

- **FFmpeg**: bundled через `fetch-ffmpeg.ps1` — BtbN GPL static build с
  NVENC/QSV/AMF/libx264. ~193 MB ffmpeg.exe + 193 MB ffprobe.exe в
  `src-tauri/binaries/ffmpeg/`. Tauri resources копируют их в bundle при
  packaging.

- **Vulkan**: `burn` + `burn-wgpu` (post-MVP, пока нет crates).
- **CoreML**: `ort` (coreml feature) — Apple Silicon, post-MVP.
- **CPU**: `ort` (default features). Везде fallback.
