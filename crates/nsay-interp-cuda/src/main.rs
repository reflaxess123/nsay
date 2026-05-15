// nsay-interp-cuda — CUDA frame interpolation sidecar (RIFE 4.9).
// All pipeline logic lives in nsay-interp-lib; this binary registers
// the CUDA execution provider.

use nsay_interp_lib::ort::execution_providers::CUDAExecutionProvider;

fn main() {
    if let Err(e) = nsay_interp_lib::run(|b| {
        // TF32 reduced-precision matmul on Ampere+ Tensor Cores. RIFE has
        // optical flow + warp ops which are matmul-heavy → meaningful win.
        // See nsay-upscale-cuda for the full rationale.
        b.with_execution_providers([
            CUDAExecutionProvider::default()
                .with_tf32(true)
                .build()
                .error_on_failure()
        ])
    }) {
        eprintln!("nsay-interp-cuda error: {e:#}");
        std::process::exit(1);
    }
}
