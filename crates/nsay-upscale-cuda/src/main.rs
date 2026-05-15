// nsay-upscale-cuda — CUDA super-resolution sidecar.
// All pipeline logic lives in nsay-upscale-lib; this binary registers
// the CUDA execution provider. `.error_on_failure()` makes silent CPU
// fallback impossible — if cuDNN is missing, we want to know loudly.

use nsay_upscale_lib::ort::execution_providers::CUDAExecutionProvider;

fn main() {
    if let Err(e) = nsay_upscale_lib::run(|b| {
        // TF32 is Ampere's reduced-precision matmul format (10-bit mantissa
        // instead of 23) — math accelerated by Tensor Cores at FP16 rates
        // while keeping FP32 dynamic range. Safe to enable globally: no
        // visible quality loss on SR tasks, ~20-40% speedup on HAT/SwinIR
        // and other matmul-heavy archs. ORT default is *off*, which is
        // strange for an inference framework — we enable it explicitly.
        // Pre-Ampere GPUs ignore the flag.
        b.with_execution_providers([
            CUDAExecutionProvider::default()
                .with_tf32(true)
                .build()
                .error_on_failure()
        ])
    }) {
        eprintln!("nsay-upscale-cuda error: {e:#}");
        std::process::exit(1);
    }
}
