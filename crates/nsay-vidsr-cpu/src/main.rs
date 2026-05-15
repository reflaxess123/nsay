// nsay-vidsr-cpu — CPU video super-resolution sidecar (RealBasicVSR).
// All pipeline logic lives in nsay-vidsr-lib; this binary picks the
// CPU device. Honest perf warning: VSR on CPU runs ~50-200x slower
// than CUDA; usable only as a correctness fallback for AMD/Intel GPUs.

use nsay_vidsr_lib::tch::Device;

fn main() {
    if let Err(e) = nsay_vidsr_lib::run(Device::Cpu) {
        eprintln!("nsay-vidsr-cpu error: {e:#}");
        std::process::exit(1);
    }
}
