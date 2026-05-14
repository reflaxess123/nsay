# nsay

Локальный AI-тулкит для картинок и видео:

- **Rembg** — удаление фона (BRIA-RMBG)
- **Upscale** — апскейл x2/x4 (Real-ESRGAN)
- **Interp** — интерполяция кадров 24→60fps (RIFE)

Tauri 2 + Svelte 5 фронт, Rust бэк с sidecar-бинарями per backend
(CUDA / DirectML / Vulkan / CoreML / CPU). Без облака.

## Quick start

```powershell
# 1. Поставь deps
cd ui && npm install && cd ..

# 2. Собери sidecars (хотя бы CPU для старта)
.\scripts\build-sidecars.ps1 -Backend cpu

# 3. Запусти dev
.\dev.cmd
```

Дальше: открой Settings → Models → скачай BRIA-RMBG. Drag-and-drop
картинку на вкладке Rembg.

## Документация

См. [CLAUDE.md](./CLAUDE.md) для архитектурного гайда.
