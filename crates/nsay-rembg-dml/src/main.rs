// nsay-rembg-dml — DirectML background-removal sidecar.
// All pipeline logic lives in nsay-rembg-lib; this binary just registers
// the DirectML execution provider and forwards errors with its own name.
//
// Works on any DirectX 12 GPU on Windows 10+ (AMD / Intel / NVIDIA).
// Requires DirectML.dll alongside the exe (staged by build-sidecars.ps1).

use nsay_rembg_lib::ort::execution_providers::DirectMLExecutionProvider;

fn main() {
    if let Err(e) = nsay_rembg_lib::run(|b| {
        b.with_execution_providers([DirectMLExecutionProvider::default().build().error_on_failure()])
    }) {
        eprintln!("nsay-rembg-dml error: {e:#}");
        std::process::exit(1);
    }
}
