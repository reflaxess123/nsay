// nsay-upscale-cuda — CUDA super-resolution sidecar.
// All pipeline logic lives in nsay-upscale-lib; this binary registers
// the CUDA execution provider. `.error_on_failure()` makes silent CPU
// fallback impossible — if cuDNN is missing, we want to know loudly.

use nsay_upscale_lib::ort::execution_providers::{CUDAExecutionProvider, TensorRTExecutionProvider};

fn main() {
    // TensorRT engine cache directory — lives outside the install tree so
    // it survives reinstalls. Engines are GPU-specific (UUID-keyed by
    // ort/TRT internally); cache misses just trigger a recompile.
    // First inference per (model, GPU, input shape range) runs the TRT
    // builder which can take 2-5 min — subsequent runs hit the cache and
    // are instant. Override via NSAY_TRT_CACHE if you want it elsewhere.
    let trt_cache = std::env::var("NSAY_TRT_CACHE").unwrap_or_else(|_| {
        let appdata = std::env::var("APPDATA").unwrap_or_else(|_| ".".to_string());
        format!("{}/nsay/trt-engines", appdata)
    });
    let _ = std::fs::create_dir_all(&trt_cache);

    if let Err(e) = nsay_upscale_lib::run(|b| {
        // EP priority: TensorRT first, then CUDA. ort tries each in order
        // and falls back silently if registration fails (TRT DLLs missing,
        // model has unsupported ops, etc.). CUDA is mandatory — its
        // .error_on_failure() makes a missing cuDNN loud instead of
        // silent CPU fallback.
        //
        // TRT options:
        //   .with_fp16(true)            — Ampere+ Tensor Cores @ 2× fp32 rate
        //   .with_engine_cache(true)    — persist compiled engines
        //   .with_engine_cache_path(p)  — where to put them
        //   .with_builder_optimization_level(3) — 0-5, 3 = balanced default
        //
        // CUDA options (see TF32 rationale in the previous commit):
        //   .with_tf32(true)            — Ampere+ TensorFloat-32 matmul
        b.with_execution_providers([
            TensorRTExecutionProvider::default()
                .with_fp16(true)
                .with_engine_cache(true)
                .with_engine_cache_path(&trt_cache)
                .with_builder_optimization_level(3)
                .build(),
            CUDAExecutionProvider::default()
                .with_tf32(true)
                .build()
                .error_on_failure(),
        ])
    }) {
        eprintln!("nsay-upscale-cuda error: {e:#}");
        std::process::exit(1);
    }
}
