// nsay-upscale-cpu — CPU super-resolution sidecar.
// All pipeline logic (file mode, streaming, tile blending) lives in
// nsay-upscale-lib; this binary just runs with no execution provider.

fn main() {
    if let Err(e) = nsay_upscale_lib::run(Ok) {
        eprintln!("nsay-upscale-cpu error: {e:#}");
        std::process::exit(1);
    }
}
