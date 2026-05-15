// nsay-rembg-cuda — CUDA background-removal sidecar.
// All pipeline logic lives in nsay-rembg-lib; this binary just registers
// the CUDA execution provider and forwards errors with its own name.
//
// Requires NVIDIA driver + CUDA 12.x + cuDNN 9.x runtime DLLs alongside
// the exe (staged by scripts/build-sidecars.ps1). `.error_on_failure()`
// makes EP-registration failures explicit instead of silently falling
// back to CPU — that bug ate days of debugging time.

use nsay_rembg_lib::ort::execution_providers::CUDAExecutionProvider;

fn main() {
    if let Err(e) = nsay_rembg_lib::run(|b| {
        b.with_execution_providers([CUDAExecutionProvider::default().build().error_on_failure()])
    }) {
        eprintln!("nsay-rembg-cuda error: {e:#}");
        std::process::exit(1);
    }
}
