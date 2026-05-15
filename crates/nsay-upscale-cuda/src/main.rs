// nsay-upscale-cuda — CUDA super-resolution sidecar.
// All pipeline logic lives in nsay-upscale-lib; this binary registers
// the CUDA execution provider. `.error_on_failure()` makes silent CPU
// fallback impossible — if cuDNN is missing, we want to know loudly.

use nsay_upscale_lib::ort::execution_providers::CUDAExecutionProvider;

fn main() {
    if let Err(e) = nsay_upscale_lib::run(|b| {
        b.with_execution_providers([CUDAExecutionProvider::default().build().error_on_failure()])
    }) {
        eprintln!("nsay-upscale-cuda error: {e:#}");
        std::process::exit(1);
    }
}
