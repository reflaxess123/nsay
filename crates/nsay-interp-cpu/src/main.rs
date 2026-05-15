// nsay-interp-cpu — CPU frame interpolation sidecar (RIFE 4.9).
// All pipeline logic lives in nsay-interp-lib; this binary just runs
// with no execution provider.

fn main() {
    if let Err(e) = nsay_interp_lib::run(Ok) {
        eprintln!("nsay-interp-cpu error: {e:#}");
        std::process::exit(1);
    }
}
