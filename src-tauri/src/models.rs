// Catalogue of downloadable ONNX models. Keep in sync with
// scripts/download-models.ps1.

use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct ModelEntry {
    pub id: &'static str,
    pub family: &'static str, // "rembg" | "upscale" | "interp"
    pub label: &'static str,
    pub url: &'static str,
    pub filename: &'static str,
    pub size_mb: u32,
    /// Optional sha256 for verification (hex, lowercase). Skipped if empty.
    /// `models_cmd::download_to` recomputes after writing and rejects on
    /// mismatch. Migration is gradual — fill in as we add models.
    pub sha256: &'static str,
    /// For `upscale` models: the model's native output:input ratio (2 or 4).
    /// The runner uses this to pick a pre-resize ratio so the user-requested
    /// scale × source = source × model_scale × ratio holds. For non-upscale
    /// families this is 0 and ignored.
    pub output_scale: u32,
    /// Preprocessing/postprocessing flavour the sidecar should use. Stays
    /// "default" until we ship a model with non-stock normalize (BEN2,
    /// BiRefNet — F2 in PLAN.md). Sidecar lib reads it via the `--preset`
    /// CLI arg, dispatches to the right preprocess/postprocess.
    pub preset: &'static str,
}

pub const CATALOG: &[ModelEntry] = &[
    // fp32 is the default — works on every EP (CPU / CUDA / DML / CoreML)
    // without dtype gymnastics. fp16 is offered for users who know they
    // want it (smaller, sometimes faster on GPU, but CPU inference for
    // fp16 is often *slower* than fp32 due to upcasting on every op).
    ModelEntry {
        id: "bria-rmbg-1.4",
        family: "rembg",
        label: "BRIA-RMBG 1.4 (fp32)",
        url: "https://huggingface.co/briaai/RMBG-1.4/resolve/main/onnx/model.onnx",
        filename: "bria-rmbg-1.4.onnx",
        size_mb: 176,
        sha256: "",
        output_scale: 0,
        preset: "bria-rmbg",
    },
    ModelEntry {
        id: "bria-rmbg-1.4-fp16",
        family: "rembg",
        label: "BRIA-RMBG 1.4 (fp16)",
        url: "https://huggingface.co/briaai/RMBG-1.4/resolve/main/onnx/model_fp16.onnx",
        filename: "bria-rmbg-1.4-fp16.onnx",
        size_mb: 88,
        sha256: "",
        output_scale: 0,
        preset: "bria-rmbg",
    },
    // Upscale catalogue. All from crj/dl-ws — same export convention
    // (input "input.1", dynamic [N, 3, H, W], pixel/255 normalisation).
    ModelEntry {
        id: "real-esrgan-x4",
        family: "upscale",
        label: "Real-ESRGAN ×4 (general)",
        url: "https://huggingface.co/crj/dl-ws/resolve/main/real_esrgan_x4.onnx",
        filename: "real-esrgan-x4.onnx",
        size_mb: 67,
        sha256: "",
        output_scale: 4,
        preset: "esrgan",
    },
    ModelEntry {
        id: "real-esrgan-x2",
        family: "upscale",
        // Native x2 weights — better quality at ×1.5/×2 than running the
        // x4 model on a downscaled source (which is what we have to do
        // when only an x4 model is installed).
        label: "Real-ESRGAN ×2 (general)",
        url: "https://huggingface.co/crj/dl-ws/resolve/main/real_esrgan_x2.onnx",
        filename: "real-esrgan-x2.onnx",
        size_mb: 67,
        sha256: "",
        output_scale: 2,
        preset: "esrgan",
    },
    ModelEntry {
        id: "real-hatgan-x4",
        family: "upscale",
        // HAT (Hybrid Attention Transformer) — alternative architecture
        // to ESRGAN. Often wins on text & fine textures, loses on photo
        // skin/foliage. Heavier (~150 MB), slower per tile.
        label: "Real-HAT-GAN ×4 (textures)",
        url: "https://huggingface.co/crj/dl-ws/resolve/main/real_hatgan_x4.onnx",
        filename: "real-hatgan-x4.onnx",
        size_mb: 153,
        sha256: "",
        output_scale: 4,
        preset: "hat",
    },
    // Frame interpolation — RIFE (Real-time Intermediate Flow Estimation).
    // 4.26 has no public ONNX export (only .pth in Practical-RIFE);
    // we'll add a self-converted 4.26 later. 4.9 from yuvraj108c is the
    // freshest production-tested ONNX export.
    // Inputs: img0 [1,3,H,W], img1 [1,3,H,W], timestep [1,1,H,W] (filled
    // with t∈[0,1]). Output: interpolated frame at time t.
    ModelEntry {
        id: "rife-4.9",
        family: "interp",
        label: "RIFE 4.9 (general)",
        url: "https://huggingface.co/yuvraj108c/rife-onnx/resolve/main/rife49_ensemble_True_scale_1_sim.onnx",
        filename: "rife-4.9.onnx",
        size_mb: 21,
        sha256: "",
        output_scale: 0,
        preset: "rife",
    },
];

pub fn find(id: &str) -> Option<&'static ModelEntry> {
    CATALOG.iter().find(|m| m.id == id)
}
