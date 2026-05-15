// nsay-interp-dml — DirectML frame interpolation sidecar (RIFE 4.9).
// All pipeline logic lives in nsay-interp-lib; this binary registers
// the DirectML execution provider.

use nsay_interp_lib::ort::execution_providers::DirectMLExecutionProvider;

fn main() {
    if let Err(e) = nsay_interp_lib::run(|b| {
        b.with_execution_providers([DirectMLExecutionProvider::default().build().error_on_failure()])
    }) {
        eprintln!("nsay-interp-dml error: {e:#}");
        std::process::exit(1);
    }
}
