# Аудит моделей для nsay: background removal, image upscale, video upscale

Дата: 2026-05-15

Этот документ фиксирует исследование без внесения изменений в код проекта. Цель: понять, какие open-source/source-available модели можно добавить для более качественного вырезания фона и апскейла изображений/видео, и отдельно разобраться, должен ли video upscale использовать отдельные video-SR модели.

## Короткий вывод

Текущий video upscale в nsay фактически является frame-by-frame image super-resolution:

- `src-tauri/src/tools/video.rs` берёт sidecar `"upscale"`, а не отдельный video-SR sidecar.
- ffmpeg декодирует видео в raw RGB.
- `nsay-upscale-*` принимает поток через `--stream`.
- каждый кадр обрабатывается независимо той же моделью, что и image upscale.
- ffmpeg кодирует результат обратно в видео и копирует audio stream.

Это нормальный быстрый и совместимый режим, но это не temporal video super-resolution. Он не использует соседние кадры, motion alignment, optical flow, recurrent state или temporal attention. Поэтому на видео возможны flicker, shimmering, плавающие детали, нестабильная текстура и разный результат на соседних кадрах.

Для качественного video upscale нужен отдельный режим и отдельная модельная ветка, условно `vidsr`: RealBasicVSR, BasicVSR++, RVRT, VRT, EDVR или другие video restoration модели. Текущий ffmpeg pipe можно оставить, но sidecar-протокол должен уметь обрабатывать окно кадров или recurrent state.

## Текущее состояние проекта

### Модели

В `models/` сейчас лежат:

- `bria-rmbg-1.4.onnx`
- `real-esrgan-x4.onnx`

В `src-tauri/src/models.rs` каталог шире:

- `bria-rmbg-1.4`
- `bria-rmbg-1.4-fp16`
- `real-esrgan-x4`
- `real-esrgan-x2`
- `real-hatgan-x4`
- `rife-4.9`

Проблемы:

- `scripts/download-models.ps1` не синхронизирован с каталогом: там нет `real-esrgan-x2`, `real-hatgan-x4`, `rife-4.9`.
- `CLAUDE.md` и `nsay.toml.example` всё ещё упоминают `rife-4.22`, а каталог содержит `rife-4.9`.
- `sha256` поля в каталоге пустые.
- `src-tauri/src/models_cmd.rs` скачивает модель во временный `.part` файл и переименовывает, но не проверяет checksum.

### Tool/backends

В `src-tauri/src/tools/mod.rs` заявлен приоритет:

```text
cuda -> dml -> vulkan -> coreml -> cpu
```

Реально в `crates/` есть:

- `nsay-rembg-cpu`
- `nsay-rembg-cuda`
- `nsay-rembg-dml`
- `nsay-upscale-cpu`
- `nsay-upscale-cuda`
- `nsay-upscale-dml`
- `nsay-interp-cpu`
- `nsay-interp-cuda`
- `nsay-interp-dml`

Vulkan/CoreML в кодовой базе как crates не найдены, хотя они есть в priority list.

### Rembg pipeline

`nsay-rembg-*` сейчас заточен под BRIA RMBG 1.4:

- fixed input 1024x1024.
- resize через `FilterType::Triangle`.
- normalize примерно как у RMBG 1.4: `pixel / 255 - 0.5`.
- берётся первый output, затем min-max normalization.
- mask resize обратно к исходному размеру.
- optional `--choke` делает binary erosion.
- итог сохраняется PNG с alpha.

Это хорошо для текущей модели, но новые модели могут требовать другой normalize, другой output layout, sigmoid, несколько outputs, dynamic input или refinement.

### Image upscale pipeline

`nsay-upscale-*` сейчас подходит для Real-ESRGAN-like ONNX:

- input tensor `[N, 3, H, W]`.
- RGB normalization `pixel / 255`.
- dynamic input поддержан.
- для больших изображений используется tiled inference:
  - `SINGLE_PASS_LIMIT = 1024`
  - `TILE_SIZE = 512`
  - `OVERLAP = 32`
  - Hann blending.
- если выбранный пользовательский scale отличается от model native scale, вход предварительно resize'ится, а модель выдаёт свой native upscale.

Это неплохая база для image SR моделей с похожим ONNX-интерфейсом, но не для diffusion SR и не для temporal video SR.

### Video upscale pipeline

`src-tauri/src/tools/video.rs`:

- вызывает `resolve_sidecar("upscale", ...)`.
- вычисляет `model_scale` из каталога модели.
- ffmpeg decode -> raw RGB stream.
- sidecar `nsay-upscale-* --stream --width --height --scale --model-scale --model`.
- ffmpeg encode -> output video, audio mux.
- encoder выбирается из NVENC/QSV/AMF/libx264.

Итог: это image SR на каждом кадре, а не video SR.

## Background removal: модели-кандидаты

### 1. BiRefNet

Источник: https://github.com/ZhengPeng7/BiRefNet

Что это:

- Bilateral Reference for High-Resolution Dichotomous Image Segmentation.
- Сильный general-purpose сегментатор foreground/background.
- Архитектурно важен: BRIA RMBG-2.0 построен на BiRefNet.

Почему интересен:

- Хороший кандидат на новый quality default для object/background cutout.
- Лучше подходит для сложных объектов и high-resolution DIS задач.
- Есть community deployment work: ONNX/TensorRT упоминаются в upstream README.

Риски:

- Нужно проверять конкретные веса и их лицензию, а не только код.
- ONNX export может потребовать отдельной подготовки.
- Нужно понять preprocess/output для каждой конкретной версии.

Рекомендация:

- Взять в benchmark как top-priority general cutout модель.
- Проверять варианты `BiRefNet`, `BiRefNet_HR`, `BiRefNet_lite`, `BiRefNet_dynamic`.

### 2. BEN2

Источник: https://huggingface.co/PramaLLC/BEN2

Что это:

- Background Erase Network v2.
- Hugging Face model card показывает license MIT.
- Есть теги ONNX, Safetensors, PyTorch.
- Заявлены hair matting, 4K processing, object segmentation, edge refinement.
- Есть пример `segment_video`.

Почему интересен:

- Очень хороший кандидат на practical default или quality mode.
- Более удобная лицензия, чем BRIA RMBG для коммерческого использования.
- Потенциально закрывает и image cutout, и video segmentation use case.

Риски:

- Full/commercial модель у авторов может отличаться от open base model.
- Нужно проверить качество base weights именно локально.
- Нужно проверить ONNX-файл, opset, input size, CUDA/DML compatibility.

Рекомендация:

- Взять в первую волну benchmark вместе с BiRefNet.
- Особенно тестировать волосы, прозрачные края, товары, сложный фон, 4K.

### 3. BRIA RMBG-2.0

Источник: https://huggingface.co/briaai/RMBG-2.0

Что это:

- Новая версия BRIA background removal.
- Model card говорит, что модель улучшает RMBG v1.4.
- Использует BiRefNet architecture.
- Input в примере 1024x1024, output - single-channel alpha matte.

Почему интересна:

- Самый прямой апгрейд текущей BRIA RMBG 1.4 ветки.
- Хорошая UX-совместимость: пользователь уже понимает этот тип модели.

Большой риск:

- Веса доступны для non-commercial use.
- Commercial use требует agreement с BRIA.
- Для продукта это юридически рискованный default.

Рекомендация:

- Можно держать как optional community/non-commercial модель, если это допустимо.
- Нельзя молча делать коммерческим default без юридической проверки.

### 4. InSPyReNet

Источник: https://github.com/plemeri/InSPyReNet

Что это:

- High-resolution salient object detection.
- MIT license.
- Старее BiRefNet/BEN2, но всё ещё полезная baseline-модель.

Почему интересна:

- Хорошая открытая альтернатива для saliency-based foreground extraction.
- Может быть легче интегрировать как fallback/fast mode.

Риски:

- Это SOD, не специализированный alpha matting.
- На волосах и полупрозрачных краях может уступать новым моделям.

Рекомендация:

- Не first priority, но стоит проверить как лёгкий permissive fallback.

### 5. MODNet

Источник: https://github.com/ZHKKKe/MODNet

Что это:

- Real-time trimap-free portrait matting.
- Apache-2.0.
- Есть ONNX/TorchScript папки в upstream.

Почему интересна:

- Отличный отдельный режим для людей: портреты, аватарки, вебкам, talking head.
- Быстрее и проще, чем большие general DIS модели.

Риски:

- Не general background remover.
- Для предметов, товаров, животных и сложных объектов не должен быть default.

Рекомендация:

- Добавить как `portrait`/`human` profile, не как замену BRIA/BEN2/BiRefNet.

### 6. Robust Video Matting

Источник: https://github.com/PeterL1n/RobustVideoMatting

Что это:

- Robust high-resolution video matting with temporal guidance.
- Recurrent neural network with temporal memory.
- Есть ONNX Runtime CPU/CUDA модели.
- По README: real-time human video matting.

Почему интересна:

- Лучший кандидат для отдельного video background removal режима для людей.
- Решает именно temporal consistency для видео.

Риск:

- GPL-3.0 license.
- Это может быть несовместимо с желаемой лицензией продукта.

Рекомендация:

- Использовать только после юридической проверки.
- Технически очень сильный reference для будущего `vid-rembg`.

### 7. SAM 2

Источник: https://github.com/facebookresearch/sam2

Что это:

- Promptable segmentation for images and videos.
- Apache-2.0/BSD-3-Clause.
- Поддерживает video memory/streaming для real-time video processing.

Почему интересна:

- Не как автоматический default cutout, а как инструмент выбора/уточнения объекта.
- Хорошо подходит для interactive mask refinement и video object tracking.

Риски:

- Без prompt/object selection это не всегда “one click background remover”.
- Интеграция UX сложнее: нужны точки, bbox, mask refinement или auto-prompt.

Рекомендация:

- Рассматривать как продвинутый режим: “выбрать объект и удалить фон/трекать в видео”.

## Image upscale: модели-кандидаты

### 1. Real-ESRGAN family

Источник: https://github.com/xinntao/Real-ESRGAN

Что уже есть:

- `real-esrgan-x4`
- `real-esrgan-x2`

Что стоит добавить:

- `RealESRGAN_x4plus_anime_6B` для anime images.
- `realesr-animevideov3` для anime videos/frames.
- `realesr-general-x4v3` как small general model с denoise strength в upstream toolchain.

Особенно важно:

- `realesr-animevideov3` называется video model, но по практической схеме Real-ESRGAN docs он применяется через extract frames -> inference -> merge frames. То есть это не полноценная temporal VSR модель, а практичная frame model, оптимизированная для anime video.

Рекомендация:

- Добавить anime/video профиль как быстрый и полезный режим.
- Не позиционировать его как temporal VSR.

### 2. Real-CUGAN

Источник: https://github.com/bilibili/ailab/tree/main/Real-CUGAN

Что это:

- Сильный anime/illustration upscaler.
- Используется в anime/video tooling.

Почему интересен:

- Хорошо закрывает мультяшные изображения, иллюстрации, line art, anime frames.
- Может давать лучший результат, чем Real-ESRGAN, на плоской графике.

Риски:

- Нужно проверить доступность ONNX/NCNN runtime под текущую архитектуру.
- Может потребовать отдельный preprocessing/runtime.

Рекомендация:

- Добавить как отдельный `anime/illustration` backend/profile.

### 3. HAT / Real-HAT-GAN

Источник: https://github.com/XPixelGroup/HAT

Что это:

- Hybrid Attention Transformer for image restoration.
- Apache-2.0.
- Есть Real_HAT_GAN_SRx4 и sharper варианты.

Что уже есть:

- В catalog nsay уже есть `real-hatgan-x4`, но download script и локальные модели отстают.

Почему интересен:

- Более качественный/sharp real-world SR режим.
- Хороший кандидат для “quality still image upscale”.

Риски:

- Тяжелее Real-ESRGAN.
- ONNX model source нужно фиксировать, проверять output и tiling.

Рекомендация:

- Сначала довести уже прописанный `real-hatgan-x4`.
- Потом benchmark against Real-ESRGAN на фото, текстурах, товарах, тексте.

### 4. SwinIR

Источник: https://github.com/JingyunLiang/SwinIR

Что это:

- Image Restoration Using Swin Transformer.
- Apache-2.0.
- Покрывает classical SR, lightweight SR, real-world image SR, denoising, JPEG artifact reduction.

Почему интересен:

- Может быть менее “галлюцинаторным”, чем GAN модели.
- Хорош для clean/restoration mode.

Риски:

- Нужно подбирать конкретные веса и экспорт.
- Качество на real-world blind SR может быть менее wow, чем GAN/diffusion.

Рекомендация:

- Добавить как `clean`/`natural` profile после Real-ESRGAN/HAT.

### 5. BSRGAN

Источник: https://github.com/cszn/BSRGAN

Что это:

- Blind image super-resolution с practical degradation model.
- Apache-2.0.

Почему интересен:

- Хороший baseline для real-world degraded images.
- Может давать более стабильный restoration, чем aggressive GAN.

Риски:

- Старее новых transformer/diffusion подходов.
- Нужно проверять ONNX и runtime.

Рекомендация:

- Хороший benchmark candidate, но не первый в очереди.

### 6. Diffusion/generative SR: SUPIR, StableSR, SeeSR, AuraSR

Источник SUPIR: https://github.com/Fanghua-Yu/SUPIR

Что это:

- Тяжелые generative/photo-realistic restoration pipelines.

Почему интересны:

- Могут давать самый впечатляющий результат на плохих фото.
- Хорошо подходят для отдельного “ultra quality” режима.

Почему не надо смешивать с текущим upscale sidecar:

- Обычно требуют PyTorch, diffusion pipeline, много VRAM.
- Не ложатся в текущий простой ONNX Runtime model interface.
- Могут галлюцинировать детали.
- Производительность и UX будут совсем другими.

Рекомендация:

- Не трогать в первом этапе.
- Рассматривать как отдельный advanced/generative module.

### 7. Face restoration

Кандидаты:

- GFPGAN
- CodeFormer
- RestoreFormer

Что это:

- Не general upscalers, а face restoration.

Почему интересны:

- После апскейла фото с людьми лицо часто выглядит хуже всего.
- Отдельная опция “restore faces” резко улучшает perceived quality.

Риски:

- Может менять идентичность лица.
- Нужны face detection, crop/merge, blending.

Рекомендация:

- Добавлять как optional postprocess, выключенный по умолчанию.

## Video upscale: отдельные video-SR модели

### Почему image model недостаточно

Image SR model видит один кадр. Она не знает:

- как двигался объект между кадрами;
- какие детали стабильны во времени;
- где compression artifact, а где реальная текстура;
- как сохранить одинаковую реконструкцию лица/текста/узора на соседних кадрах.

Video SR model обычно использует:

- соседние кадры;
- optical flow или deformable alignment;
- temporal attention;
- recurrent propagation;
- hidden state;
- clip/window processing.

Итоговая разница:

- меньше flicker;
- стабильнее детали;
- лучше восстановление мелких объектов;
- лучше compressed/noisy video restoration;
- дороже по памяти и runtime.

### 1. RealBasicVSR

Источник: https://github.com/ckkelvinchan/RealBasicVSR

Что это:

- Real-world video super-resolution.
- CVPR 2022.
- Apache-2.0.

Почему первый кандидат:

- Именно real-world VSR, а не только bicubic benchmark.
- Лицензия лучше, чем у VRT/RVRT.
- Хороший баланс качества и практичности.

Риски:

- Оригинальный runtime PyTorch/MMEditing-like.
- ONNX export может быть сложнее, чем у image SR.

Рекомендация:

- Первый POC для `vidsr`.
- Сначала проверить runtime path: PyTorch/libtorch/ONNX/ncnn.

### 2. BasicVSR++

Источник: https://github.com/ckkelvinchan/BasicVSR_PlusPlus

Что это:

- Video SR with enhanced propagation and alignment.
- CVPR 2022.
- Apache-2.0.

Почему интересен:

- Сильная академическая и практическая база.
- Подходит как high-quality temporal SR baseline.

Риски:

- Сложный runtime.
- Может быть тяжелее и менее convenient для desktop app.

Рекомендация:

- Второй кандидат после RealBasicVSR.

### 3. RVRT

Источник: https://github.com/JingyunLiang/RVRT

Что это:

- Recurrent Video Restoration Transformer.
- Покрывает video SR, deblurring, denoising.

Почему интересен:

- Сильная temporal модель.
- Использует local neighboring frames плюс recurrent framework.

Риск:

- CC-BY-NC license.
- Тяжелее в интеграции.

Рекомендация:

- Хорош для research/benchmark, но не commercial default.

### 4. VRT

Источник: https://github.com/JingyunLiang/VRT

Что это:

- Video Restoration Transformer.
- Покрывает video SR/restoration задачи.

Почему интересен:

- Очень сильный quality candidate.

Риск:

- CC-BY-NC license.
- Тяжелый runtime и memory.

Рекомендация:

- Не первый practical candidate. Использовать для benchmark/reference.

### 5. EDVR / BasicSR / MMagic

Источник BasicSR: https://github.com/XPixelGroup/BasicSR

Что это:

- Open-source image/video restoration toolbox.
- Содержит EDVR, BasicVSR, SwinIR, ESRGAN и другие модели.
- Apache-2.0 для BasicSR.

Почему интересен:

- Хороший источник reference implementations.
- EDVR старее, но зрелый.

Риски:

- Не всегда просто превратить в лёгкий desktop sidecar.

Рекомендация:

- Использовать как источник baseline и reference code.

## Архитектурные рекомендации

### 1. Развести image upscale и temporal video upscale

Оставить текущий режим:

```text
vid upscale fast = frame-by-frame image SR
```

Добавить отдельный режим:

```text
vid upscale temporal/quality = true video SR
```

На уровне tools:

```text
upscale  - image SR + fast per-frame video SR
vidsr    - temporal video SR
interp   - frame interpolation / slow motion
rembg    - image background removal
vidrembg - future video background removal / matting
```

### 2. Новый sidecar protocol для VSR

Текущий `--stream` подразумевает:

```text
read one frame -> process one frame -> write one frame
```

Для VSR нужно одно из двух:

Sliding window:

```text
read N frames -> infer center frame(s) -> slide -> write stable output
```

Recurrent:

```text
read frame -> update hidden state -> write frame
```

Для ffmpeg это всё ещё может выглядеть как raw RGB stdin/stdout, но внутри sidecar нужен буфер и latency.

### 3. Добавить model metadata

Текущего `family`, `filename`, `output_scale` недостаточно.

Нужны поля:

```text
task: rembg | upscale | vidsr | interp | vidrembg
runtime: onnx | torch | ncnn | tensorrt | directml
license: apache-2.0 | mit | gpl-3.0 | cc-by-nc | custom
commercial_allowed: true | false | unknown
input_layout: nchw | nhwc
input_color: rgb | bgr
input_size: fixed | dynamic
normalize: preset name
output_type: alpha | rgb | frames | flow | logits
temporal: false | window | recurrent
native_scale: 1 | 2 | 3 | 4
tags: photo | anime | portrait | product | video | fast | quality
```

### 4. Разделить “source-available” и реально permissive open-source

Важно:

- BRIA RMBG 1.4/2.0 не стоит считать полностью свободными для коммерческого продукта.
- RVM технически хорош, но GPL-3.0.
- VRT/RVRT сильные, но CC-BY-NC.
- BEN2, MODNet, InSPyReNet, SwinIR, BSRGAN, HAT, RealBasicVSR, BasicVSR++ выглядят лицензированно проще, но конкретные weights всё равно нужно проверять отдельно.

### 5. Benchmark перед добавлением в UI

Нужен фиксированный test corpus:

Background removal:

- волосы на сложном фоне;
- товар на белом/сером фоне;
- прозрачные/полупрозрачные объекты;
- животные;
- тонкие детали: провода, листья, ремешки;
- несколько объектов;
- текст/логотипы рядом с объектом;
- 4K фото.

Image upscale:

- low-res portrait;
- e-commerce product;
- текст/скриншот;
- anime frame;
- line art;
- compressed JPEG;
- noisy phone photo;
- small face.

Video upscale:

- low-bitrate 720p -> 1080p/4K;
- anime clip;
- talking head;
- fast motion;
- camera pan;
- thin lines/text;
- dark/noisy video;
- compression artifacts.

Метрики:

- speed FPS;
- peak RAM/VRAM;
- file size;
- visual artifacts;
- edge quality;
- temporal flicker;
- license status;
- ONNX/DML/CUDA compatibility.

## Рекомендуемая дорожная карта

### Этап 0: привести текущую базу в порядок

Без новых моделей:

- синхронизировать `models.rs`, `download-models.ps1`, `CLAUDE.md`, `nsay.toml.example`;
- заполнить `sha256`;
- добавить license metadata;
- явно показать в UI, что текущий video upscale - fast frame-by-frame mode;
- проверить, нужны ли `vulkan/coreml` в priority list, если crates отсутствуют.

### Этап 1: rembg benchmark

Кандидаты:

- BRIA RMBG 1.4 current baseline;
- BEN2;
- BiRefNet;
- BRIA RMBG 2.0, если допустима non-commercial модель;
- MODNet только для portrait;
- InSPyReNet как permissive fallback.

Вероятный результат:

- default general: BEN2 или BiRefNet;
- optional BRIA 2.0: только с явным license warning;
- portrait fast: MODNet;
- future video human matting: RVM или BEN2 video path после проверки лицензии.

### Этап 2: image upscale additions

Сначала закрыть простые профили:

- Real-ESRGAN x2/x4 cleanup;
- Real-HAT-GAN x4 довести до рабочего состояния;
- RealESRGAN animevideov3;
- Real-CUGAN;
- SwinIR или BSRGAN как clean/natural mode.

Не добавлять diffusion SR в этот этап.

### Этап 3: настоящий video SR POC

Первый POC:

- RealBasicVSR.

Второй:

- BasicVSR++.

Research-only/quality reference:

- RVRT;
- VRT.

Нужно решить runtime:

- ONNX Runtime, если export проходит и ops поддержаны;
- libtorch/PyTorch sidecar, если ONNX слишком болезненный;
- ncnn, если есть готовый стабильный inference path.

### Этап 4: advanced modes

После базовой стабильности:

- face restoration postprocess;
- SAM2 interactive/object video masks;
- video background removal/matting;
- diffusion/generative SR как отдельный heavy quality module.

## Приоритетный shortlist

### Добавлять/тестировать первыми

Background removal:

1. BEN2
2. BiRefNet
3. MODNet для portrait
4. BRIA RMBG-2.0 только после license decision

Image upscale:

1. RealESRGAN animevideov3
2. Real-CUGAN
3. Real-HAT-GAN x4, уже есть в catalog
4. SwinIR или BSRGAN

Video upscale:

1. RealBasicVSR
2. BasicVSR++
3. RVRT/VRT только benchmark/research из-за CC-BY-NC

## Источники

- BRIA RMBG 1.4: https://huggingface.co/briaai/RMBG-1.4
- BRIA RMBG 2.0: https://huggingface.co/briaai/RMBG-2.0
- BiRefNet: https://github.com/ZhengPeng7/BiRefNet
- BEN2: https://huggingface.co/PramaLLC/BEN2
- InSPyReNet: https://github.com/plemeri/InSPyReNet
- MODNet: https://github.com/ZHKKKe/MODNet
- Robust Video Matting: https://github.com/PeterL1n/RobustVideoMatting
- SAM 2: https://github.com/facebookresearch/sam2
- Real-ESRGAN: https://github.com/xinntao/Real-ESRGAN
- Real-ESRGAN anime video models: https://github.com/xinntao/Real-ESRGAN/blob/master/docs/anime_video_model.md
- Real-CUGAN: https://github.com/bilibili/ailab/tree/main/Real-CUGAN
- HAT: https://github.com/XPixelGroup/HAT
- SwinIR: https://github.com/JingyunLiang/SwinIR
- BSRGAN: https://github.com/cszn/BSRGAN
- SUPIR: https://github.com/Fanghua-Yu/SUPIR
- RealBasicVSR: https://github.com/ckkelvinchan/RealBasicVSR
- BasicVSR++: https://github.com/ckkelvinchan/BasicVSR_PlusPlus
- RVRT: https://github.com/JingyunLiang/RVRT
- VRT: https://github.com/JingyunLiang/VRT
- BasicSR: https://github.com/XPixelGroup/BasicSR
