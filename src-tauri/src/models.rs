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
    /// Optional sha256 for verification (hex). Skipped if empty.
    pub sha256: &'static str,
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
    },
    ModelEntry {
        id: "bria-rmbg-1.4-fp16",
        family: "rembg",
        label: "BRIA-RMBG 1.4 (fp16)",
        url: "https://huggingface.co/briaai/RMBG-1.4/resolve/main/onnx/model_fp16.onnx",
        filename: "bria-rmbg-1.4-fp16.onnx",
        size_mb: 88,
        sha256: "",
    },
    ModelEntry {
        id: "real-esrgan-x4plus",
        family: "upscale",
        label: "Real-ESRGAN x4plus",
        url: "https://huggingface.co/qualcomm/Real-ESRGAN-x4plus/resolve/main/Real-ESRGAN-x4plus.onnx",
        filename: "real-esrgan-x4plus.onnx",
        size_mb: 67,
        sha256: "",
    },
];

pub fn find(id: &str) -> Option<&'static ModelEntry> {
    CATALOG.iter().find(|m| m.id == id)
}
