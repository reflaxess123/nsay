// nsay-rembg-cpu — CPU background-removal sidecar.
// All pipeline logic lives in nsay-rembg-lib; this binary just registers
// the (lack of) execution provider and forwards errors with its own name.

fn main() {
    if let Err(e) = nsay_rembg_lib::run(Ok) {
        eprintln!("nsay-rembg-cpu error: {e:#}");
        std::process::exit(1);
    }
}
