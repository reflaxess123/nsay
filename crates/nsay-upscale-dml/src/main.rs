// nsay-upscale-dml — DirectML super-resolution sidecar.
// All pipeline logic lives in nsay-upscale-lib; this binary registers
// the DirectML execution provider. Works on any DX12 GPU on Windows 10+.

use nsay_upscale_lib::ort::execution_providers::DirectMLExecutionProvider;

fn main() {
    if let Err(e) = nsay_upscale_lib::run(|b| {
        b.with_execution_providers([DirectMLExecutionProvider::default().build().error_on_failure()])
    }) {
        eprintln!("nsay-upscale-dml error: {e:#}");
        std::process::exit(1);
    }
}
