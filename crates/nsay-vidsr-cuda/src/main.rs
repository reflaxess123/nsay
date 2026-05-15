// nsay-vidsr-cuda — CUDA video super-resolution sidecar (RealBasicVSR).
// All pipeline logic lives in nsay-vidsr-lib; this binary picks the
// first CUDA device. Falls back to a clear error if libtorch reports
// no CUDA devices, instead of silently slipping onto CPU.

use nsay_vidsr_lib::tch::{utils, Device};

fn main() {
    if !utils::has_cuda() {
        eprintln!("nsay-vidsr-cuda error: libtorch reports no CUDA device");
        std::process::exit(1);
    }
    if let Err(e) = nsay_vidsr_lib::run(Device::Cuda(0)) {
        eprintln!("nsay-vidsr-cuda error: {e:#}");
        std::process::exit(1);
    }
}
