# nsay — Roadmap по моделям (по итогам codex_audit.md)

Дата: 2026-05-15
Источник: [`codex_audit.md`](codex_audit.md)

Главный вывод аудита: текущий **video upscale = frame-by-frame image SR**. Это не temporal VSR, отсюда flicker и shimmering. Чтобы починить — нужен отдельный tool `vidsr` с новым sidecar-протоколом (sliding window / recurrent state).

Вторичные выводы: каталог рассинхронизирован с download-script и docs, нет sha256, нет license metadata, BRIA RMBG = non-commercial и не должен быть тихим default'ом.

---

## Tracker

Стейтус по фазам — отметка `[x]` ставится по факту merge.

- [ ] **F0** — Чистка текущей базы (без новых моделей)
- [ ] **F1** — Метаданные модели: license, runtime, normalize, sha256
- [ ] **F2** — Rembg: добавить BEN2 + BiRefNet + MODNet
- [ ] **F3** — Image upscale: anime/HAT/SwinIR
- [ ] **F4** — `vidsr` tool: новый sidecar-протокол + RealBasicVSR POC
- [ ] **F5** — Advanced: face restore + SAM2 + video matting

---

## F0. Чистка (S, ~день)

Цель: устранить рассинхрон между `models.rs`, `download-models.ps1`, `CLAUDE.md`, `nsay.toml.example`. Никаких новых моделей.

### F0.1 Sync `download-models.ps1` с каталогом

Файл: `scripts/download-models.ps1`. Сейчас содержит только 3 entries: `bria-rmbg-1.4`, `bria-rmbg-1.4-fp16`, `real-esrgan-x4`. Каталог в `src-tauri/src/models.rs:23` содержит ещё `real-esrgan-x2`, `real-hatgan-x4`, `rife-4.9`.

Действие: дописать недостающие entries. Лучше — **сгенерировать `$catalog` из `models.rs`** через `cargo run --bin nsay-models-dump` (новая отдельная утилита) или просто завести JSON-файл `models.json` как single source of truth, читаемый и Rust'ом, и PowerShell'ом. Compile-time include через `include_str!` / `serde_json`.

### F0.2 Fix `nsay.toml.example`

Файл: `nsay.toml.example:21`. Сейчас `model = "rife-4.22"` — такого id нет в каталоге, есть `rife-4.9`.

### F0.3 Fix `CLAUDE.md`

Заменить упоминания `rife-4.22` на `rife-4.9` в разделе "Модели" (строки про "interp"). Также упомянуть честно: video upscale = per-frame, не temporal.

### F0.4 Vulkan/CoreML в priority list

Файл: `src-tauri/src/tools/mod.rs:14`. `BACKEND_PRIORITY = [cuda, dml, vulkan, coreml, cpu]`, но crates `nsay-*-vulkan` и `nsay-*-coreml` отсутствуют. `resolve_sidecar` это переживёт (просто пропустит несуществующие), но UI в `SidePanel.svelte` показывает их как кликабельные — ввод в заблуждение.

Действие: оставить в priority list (это roadmap), но в SidePanel сделать невидимыми (не disabled, а скрытыми) backends, для которых нет ни одного crate. Условие — `available_backends_any()` уже возвращает фактический список.

### F0.5 UI honesty: пометить vid-upscale как "fast frame-by-frame"

Файл: `ui/src/routes/upscale/+page.svelte` (видеo-вкладка) и `ui/src/lib/state/vidUpscale.svelte.ts`. Добавить под dropzone короткую подпись: _"Fast mode · кадр-за-кадром, без temporal smoothing"_. Чтобы пользователь не ждал чудес и понимал, что `vidsr` придёт отдельным tab'ом.

---

## F1. Метаданные модели (M, ~1-2 дня)

Цель: подложить рельсы под лицензионную честность и под расширение каталога. Без этого следующие фазы будут болезненно лепиться сверху.

### F1.1 Расширить `ModelEntry`

Файл: `src-tauri/src/models.rs:7`. Добавить поля:

```rust
pub struct ModelEntry {
    // ...existing fields...
    pub license: &'static str,           // "apache-2.0" | "mit" | "gpl-3.0" | "cc-by-nc" | "bria-noncommercial"
    pub commercial_allowed: bool,
    pub runtime: &'static str,           // "onnx" | "torch" — на будущее
    pub input_layout: &'static str,      // "nchw" | "nhwc"
    pub input_color: &'static str,       // "rgb" | "bgr"
    pub input_size: &'static str,        // "fixed:1024x1024" | "dynamic" | "multiple-of-32"
    pub normalize: &'static str,         // preset: "div255" | "div255-sub0.5" | "imagenet"
    pub output_type: &'static str,       // "alpha" | "rgb" | "frames" | "logits"
    pub temporal: &'static str,          // "none" | "window" | "recurrent"
    pub tags: &'static [&'static str],   // ["photo","anime","portrait","product","video","fast","quality"]
}
```

### F1.2 sha256 verification

Файл: `src-tauri/src/models_cmd.rs:192` (`download_to`). Сейчас sha256 поле есть в каталоге, но игнорируется.

Действие: после `rename(tmp, dest)` — если `entry.sha256` непустой, посчитать sha256 файла (`sha2` уже в зависимостях?) и сравнить. На mismatch — удалить файл и вернуть error. Не падать, если sha256 пустой (миграция постепенная).

### F1.3 License gate в UI

Файл: `ui/src/lib/components/SidePanel.svelte`. Если `commercial_allowed === false` — показывать рядом с моделью маленький badge "non-commercial" и при первом выборе модели — тонкий warning toast. Без блокировки скачивания. Как минимум — не делать BRIA молчаливым default'ом для новых пользователей.

### F1.4 Заполнить sha256 для всех существующих

Реальный hash берётся `Get-FileHash -Algorithm SHA256 models\<file>.onnx` после первого успешного download. Скрипт `scripts/refresh-sha256.ps1` который обновит `models.rs` через `toml_edit`-стиль regex (или ручное).

---

## F2. Rembg расширение (M-L, ~3-5 дней)

Цель: дать пользователю выбор моделей кроме BRIA. Особенно важно на portrait и на сложных краях.

### F2.1 BEN2 (priority #1)

- Source: `https://huggingface.co/PramaLLC/BEN2`
- License: MIT — **commercial-allowed**, можно делать default
- Каталог: `id: "ben2-base"`, family: `rembg`, normalize: TBD (проверить model card / inference notebook)
- Преимущества: волосы, 4K, edge refinement, заявлен video segmentation
- Sidecar: текущий `nsay-rembg-*` совместим, если input/output форматы те же. Иначе — добавить `--preset ben2` который переключает normalize/postprocess внутри `crates/nsay-rembg-cpu/src/main.rs`.

Действие:
1. Скачать ONNX вручную, проверить input shape/dtype через `python -c "import onnx; ..."`.
2. Прогнать на 5 reference картинках через standalone Python — зафиксировать ground-truth.
3. Запустить через текущий sidecar — сравнить mask diff. Если совпадает с точностью до пары процентов — добавить в каталог. Если нет — расширить sidecar поддержкой ben2 preset.
4. Добавить в `models.rs` с правильной license metadata.

### F2.2 BiRefNet (priority #2)

- Source: `https://github.com/ZhengPeng7/BiRefNet`
- License: MIT (проверить конкретные веса)
- Варианты: `BiRefNet`, `BiRefNet_HR`, `BiRefNet_lite`. Лёгкий — добавить первым.
- Sidecar: вероятно нужен новый preset (другой normalize, другой output layout — может быть logits после sigmoid).

### F2.3 MODNet (portrait)

- Source: `https://github.com/ZHKKKe/MODNet`, ONNX готов в upstream
- License: Apache-2.0
- Позиционирование: **portrait-only** profile, не general default. В UI помечать tag "portrait" и не показывать как первый выбор.

### F2.4 Refactor sidecar под несколько presets

Файл: `crates/nsay-rembg-{cpu,cuda,dml}/src/main.rs`. Сейчас hardcoded под BRIA RMBG 1.4 (1024×1024, normalize `pixel/255 - 0.5`). Когда добавим ≥2 модели с разным preprocess — нужен `--preset {bria-rmbg-1.4|ben2|birefnet|modnet}` arg, маппящийся внутри в normalize/postprocess. Альтернатива: засунуть preset в metadata и передавать `--input-size`, `--normalize-mean`, `--normalize-std` параметрами — но это менее robust (легче ошибиться).

Рекомендация: enum-preset, лежит рядом с моделью в каталоге (`models.rs` поле `preset`).

### F2.5 BRIA RMBG-2.0 (опционально, с warning)

Только если решим держать non-commercial модели. License: BRIA non-commercial — **в UI обязательно warning**.

---

## F3. Image upscale расширение (M, ~3-4 дня)

### F3.1 Real-ESRGAN anime models

- `realesr-animevideov3` — лёгкий, для anime frames
- `RealESRGAN_x4plus_anime_6B` — полный anime
- ONNX доступны на HF mirror `crj/dl-ws` или конвертируем сами

Sidecar: совместим с текущим `nsay-upscale-*` (тот же ESRGAN-style intf), просто новые entries в каталоге с tag `["anime"]`.

### F3.2 Real-CUGAN

- Source: `bilibili/ailab/Real-CUGAN`
- Anime/illustration upscaler
- Возможно потребуется отдельный preset (другой normalize / другой output)

### F3.3 Real-HAT-GAN x4 — довести до рабочего

Уже в каталоге (`models.rs:74`) но в `download-models.ps1` нет. После F0.1 это решается. Прогнать benchmark и зафиксировать sha256.

### F3.4 SwinIR

- Apache-2.0
- "Cleaner" mode, без галлюцинаций GAN
- Несколько вариантов: classical SR, lightweight SR, real-world SR. Брать real-world.

### F3.5 Tag-based UI filter

В `SidePanel.svelte` добавить chip-фильтр: `[All] [Photo] [Anime] [Quality] [Fast]`. Фильтрует по `tags`. Снимает confusion от длинного списка моделей.

---

## F4. Настоящий video SR — `vidsr` tool (L, ~1-2 недели)

Самая жирная фаза. Runtime decision **зафиксирован** на основе deep research (см. PLAN v2 round): **libtorch (tch-rs)**, не ONNX. Все production VSR деплои (vs-basicvsrpp, Replicate) идут через PyTorch напрямую — `mmcv:grid_sampler` + `deform_conv2d` не registered в ORT, [open-mmlab/mmagic#1004](https://github.com/open-mmlab/mmagic/issues/1004) closed без resolution. ncnn тоже не катит — PNNX не имеет `torchvision.ops.deform_conv2d`.

DML опускаем для VSR (libtorch не имеет DML EP) → AMD/Intel юзеры на CPU vidsr с честным warning.

### F4.0 Setup (✅ DONE — scripts ready, но не запущены)

- ✅ `scripts/fetch-libtorch.ps1` — качает официальный libtorch zip (CUDA 12.4 ~2.3 GB или CPU ~180 MB), стейджит в `src-tauri/binaries/libtorch/`. Запуск: `.\scripts\fetch-libtorch.ps1` или `-Cpu` для лёгкого CPU билда.
- ✅ `scripts/convert-realbasicvsr.py` — Python template, конвертит RealBasicVSR `.pth` через `torch.jit.trace` → `.pt`. Требует `pip install torch mmcv-full mmagic` в отдельном venv, плюс скачанный official `RealBasicVSR_x4.pth` с GitHub releases.

### F4.1 Новый tool `vidsr` (после F4.0 запущен)

Файлы:
- `src-tauri/src/tools/mod.rs`: добавить `"vidsr"` в `TOOLS`
- `src-tauri/src/tools/vidsr.rs`: новый runner (копия `video.rs`, но spawn'ит `nsay-vidsr-*`, не `nsay-upscale-*`)
- `crates/nsay-vidsr-{cpu,cuda}/`: новые crates на `tch = "0.24"` (без -dml — libtorch не имеет DirectML EP)
- `crates/nsay-vidsr-lib/`: shared pipeline (window buffer + tch inference)

UI:
- Новый tab "vid SR (quality)" в `+layout.svelte` рядом с "vid upscale"
- Старый "vid upscale" остаётся как **fast** mode с явной подписью
- DML/AMD/Intel юзеры на vidsr tab видят honest warning "CPU only — slow"

### F4.2 Sidecar-протокол с буферизацией

Текущий `--stream` делает one-frame-in / one-frame-out. Для VSR это не подходит. Два варианта (выбираем по модели):

**Sliding window** (RealBasicVSR в classical inference):

```
read N frames into buffer
infer → write center frame
slide window by 1
```

Latency: N/2 frames. Память: N×W×H×3 байт. Для 1080p × N=15 = ~93 MB.

**Recurrent** (BasicVSR++ оригинальный mode, опционально):

```
read frame → update hidden state → write upscaled frame
```

Latency: 0. Память: hidden state (~100 MB) живёт между кадрами.

Протокол args: `--stream --width W --height H --scale K --window N` или `--recurrent`. Stdin/stdout формат тот же raw RGB — ffmpeg pipe не меняется. Прогресс по stderr `frame N` (как сейчас, namespaced `vid-vidsr-progress`).

### F4.3 RealBasicVSR POC (после F4.0 запущен пользователем)

- Source: `https://github.com/ckkelvinchan/RealBasicVSR`
- License: Apache-2.0 (по нашему правилу — игнорим лицензии)
- Runtime: **libtorch через tch-rs**. ONNX путь подтверждённо мёртв.
- Workflow:
  1. Пользователь запускает `fetch-libtorch.ps1` (один раз, ~2.3 GB)
  2. Пользователь скачивает `RealBasicVSR_x4.pth` с GH releases
  3. Пользователь запускает `convert-realbasicvsr.py` → получает `.pt`
  4. Я (или будущая сессия) пишу `crates/nsay-vidsr-cuda/` с `tch::CModule::load`, sliding-window inference, raw RGB pipe protocol
  5. tools/vidsr.rs Tauri runner + UI tab
- Cost ranking pick-wrong: ORT-stay (worst) > ncnn (impossible) > **tch-rs (best)**

### F4.4 BasicVSR++ (опционально, после RealBasicVSR работает)

Recurrent path. Тяжелее (deformable conv), но качество выше. Тот же tch-rs runtime — переиспользует инфраструктуру F4.1-3.

### F4.4 BasicVSR++ (после RealBasicVSR работает)

Recurrent path. Тяжелее, но качество выше. Опционален.

### F4.5 RVRT/VRT — **не добавлять в продукт**

CC-BY-NC license. Только для internal benchmark, чтобы понимать ceiling качества.

### F4.6 Honest UI labels

Под каждой VSR-моделью указывать: `~X fps на 720p · ~Y GB VRAM`. Иначе пользователь запустит на 4K и убьёт систему.

---

## F5. Advanced modes (отдельный спринт, после F4)

Не блокирует основную работу.

### F5.1 Face restoration postprocess

- GFPGAN или CodeFormer
- Опциональный шаг **после** upscale
- Toggle "Restore faces" в UI, default off (может менять идентичность лица)
- Pipeline: upscale → face detect → crop → restore → blend back

### F5.2 SAM2 для interactive cutout

- Apache-2.0/BSD-3
- Отдельный mode "Manual select" в rembg: пользователь кликает по объекту, SAM2 даёт mask
- Для видео: трекинг объекта по кадрам
- Большой UI lift — отложить пока F2-F4 не стабильны

### F5.3 Video background removal (`vidrembg`)

- BEN2 video path (если лицензия позволит)
- RVM — отлично, но **GPL-3.0** → отдельная сборка / opt-in / не в основной билд

### F5.4 Diffusion SR (SUPIR/StableSR)

- Heavy, требует libtorch + diffusion pipeline
- **Не вписывается** в текущий ONNX sidecar
- Только если будет отдельный "advanced/heavy" модуль; не в первой волне

---

## Чеклист для каждой новой модели

Прежде чем коммитить новую entry в `models.rs`:

- [ ] License проверена и зафиксирована в `license` поле
- [ ] `commercial_allowed` выставлен честно
- [ ] sha256 снят с downloaded файла
- [ ] Препроцесс/нормализация задокументированы (`normalize` preset)
- [ ] Прогон на reference set (5+ картинок/видео) — diff с upstream Python pipeline
- [ ] Бенчмарк FPS / peak RAM / VRAM на dev машине
- [ ] Проверка на CUDA, DML, CPU отдельно (минимум CPU+1 GPU backend)
- [ ] UI tag(s) добавлены
- [ ] `download-models.ps1` обновлён (или generated)

---

## Бенчмарк-corpus

Завести `benchmark/` (gitignore'нутый) с фиксированным набором. Каждый PR с моделью прикладывает diff visual в PR description. Без corpus'а добавление моделей — рулетка.

**Rembg** (8 шт):
- волосы на сложном фоне
- товар на белом
- прозрачный/полупрозрачный объект
- животное
- тонкие провода/листья
- несколько объектов
- текст рядом с объектом
- 4K фото

**Image upscale** (8 шт):
- low-res portrait
- e-commerce product
- скриншот/текст
- anime frame
- line art
- compressed JPEG
- noisy phone photo
- маленькое лицо

**Video** (8 клипов по 3-5 сек):
- low-bitrate 720p
- anime
- talking head
- fast motion
- camera pan
- thin lines/text
- dark/noisy
- compression artifacts

---

## Приоритеты — что делать сначала

Если время ограничено — порядок такой:

1. **F0** (день) — без этого все docs/scripts врут
2. **F1** (1-2 дня) — без metadata следующие фазы превратятся в спагетти
3. **F2.1 BEN2** (1-2 дня) — самый высокий ROI: лучший cutout, MIT license, можно сразу делать default
4. **F4.1-F4.3 RealBasicVSR POC** (3-7 дней) — главный architectural blocker; пока не проверен ONNX export, video SR висит в воздухе
5. Всё остальное — по мере необходимости

---

## Что **не** делать

- Не трогать diffusion SR (SUPIR/StableSR) пока — другой runtime, другой UX
- Не делать BRIA RMBG 2.0 default — non-commercial license
- Не коммитить RVM в основной билд — GPL-3.0 заразит весь проект
- Не добавлять модели без sha256 и license metadata после F1
- Не складывать `vidsr` в тот же tool что `upscale` — protocol несовместим
