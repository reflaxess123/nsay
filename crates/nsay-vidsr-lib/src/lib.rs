// Shared video super-resolution pipeline (RealBasicVSR via tch-rs).
// Backend bins (nsay-vidsr-cpu / -cuda) call `run(device)` with a
// `tch::Device` — that's the one part that actually differs.
//
// Why libtorch instead of ORT: RealBasicVSR / BasicVSR++ rely on
// `mmcv:grid_sampler` + `deform_conv2d`, neither of which exports
// cleanly to ONNX (open-mmlab/mmagic#1004 — closed without resolution).
// libtorch ships the original PyTorch op kernels so the model loads as
// a TorchScript .pt and runs unmodified.
//
// No DirectML variant: libtorch has no DML EP. AMD/Intel GPU users on
// the vidsr tab fall back to CPU and the UI flags this honestly.
//
// Streaming protocol (chunked, not sliding-window — chunks are simpler
// to reason about and the visual difference is small at chunk=15):
//
//   args:   --model <pt_path> --stream --width W --height H --scale K
//           [--window N]            ; default N=15 (RealBasicVSR clip size)
//   stdin:  raw RGB frames, N×W×H×3 bytes per chunk
//   stdout: raw RGB frames, N×(W*K)×(H*K)×3 bytes per chunk
//   stderr: `stream-ready` once, then `frame M` per emitted output frame
//   exit:   0 success, 1 failure
//
// On EOF we pad the last partial chunk by repeating the final frame so
// the model still sees a full clip, then emit only the real frames.

use std::io::{Read, Write};
use std::path::PathBuf;

use anyhow::{bail, Context, Result};

pub use tch;
use tch::{CModule, Device, Kind, Tensor};

pub struct Args {
    pub model: PathBuf,
    pub width: u32,
    pub height: u32,
    pub scale: u32,
    pub window: u32,
}

pub fn parse_args() -> Result<Args> {
    let mut model: Option<PathBuf> = None;
    let mut width: u32 = 0;
    let mut height: u32 = 0;
    let mut scale: u32 = 4;
    let mut window: u32 = 15;
    let mut stream = false;

    let mut iter = std::env::args().skip(1);
    while let Some(a) = iter.next() {
        match a.as_str() {
            "--model"  => model  = Some(PathBuf::from(iter.next().context("--model needs a value")?)),
            "--width"  => width  = iter.next().context("--width needs a value")?.parse()?,
            "--height" => height = iter.next().context("--height needs a value")?.parse()?,
            "--scale"  => scale  = iter.next().context("--scale needs a value")?.parse()?,
            "--window" => window = iter.next().context("--window needs a value")?.parse()?,
            "--stream" => stream = true,
            other => bail!("unknown argument: {}", other),
        }
    }
    if !stream { bail!("--stream is required (vidsr is video-only)"); }
    if width == 0 || height == 0 { bail!("--width and --height required"); }
    if !(2..=4).contains(&scale) { bail!("--scale must be 2..=4, got {}", scale); }
    if !(3..=30).contains(&window) { bail!("--window must be 3..=30, got {}", window); }

    Ok(Args { model: model.context("--model is required")?, width, height, scale, window })
}

fn emit_progress(n: u64) {
    let _ = writeln!(std::io::stderr(), "frame {}", n);
    let _ = std::io::stderr().flush();
}

/// Run the chunked VSR pipeline on the requested device. Backend bins:
/// `Device::Cuda(0)` for nsay-vidsr-cuda, `Device::Cpu` for nsay-vidsr-cpu.
pub fn run(device: Device) -> Result<()> {
    let args = parse_args()?;
    if !args.model.exists() {
        bail!("model not found: {}", args.model.display());
    }

    let mut module = CModule::load_on_device(&args.model, device)
        .with_context(|| format!("CModule::load_on_device: {}", args.model.display()))?;
    module.set_eval();

    let w = args.width as i64;
    let h = args.height as i64;
    let n = args.window as i64;
    let s = args.scale as i64;
    let frame_bytes = (args.width as usize) * (args.height as usize) * 3;
    let chunk_bytes = frame_bytes * args.window as usize;

    let _ = writeln!(std::io::stderr(), "stream-ready");
    let _ = std::io::stderr().flush();

    let mut stdin = std::io::stdin().lock();
    let mut stdout = std::io::stdout().lock();
    let mut chunk_buf = vec![0u8; chunk_bytes];
    let mut emitted: u64 = 0;

    loop {
        // Read up to N frames; track how many real frames we got so we
        // can crop padding off the output.
        let mut filled: usize = 0;
        while filled < args.window as usize {
            let off = filled * frame_bytes;
            match stdin.read_exact(&mut chunk_buf[off..off + frame_bytes]) {
                Ok(()) => filled += 1,
                Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
                Err(e) => return Err(anyhow::anyhow!("stdin read failed: {e}")),
            }
        }
        if filled == 0 { break; }

        // Pad the tail of the chunk by repeating the last real frame so
        // the model still receives a full clip (RealBasicVSR's inference
        // graph is traced at fixed window size).
        if filled < args.window as usize {
            let last_off = (filled - 1) * frame_bytes;
            let last_frame: Vec<u8> = chunk_buf[last_off..last_off + frame_bytes].to_vec();
            for i in filled..args.window as usize {
                let off = i * frame_bytes;
                chunk_buf[off..off + frame_bytes].copy_from_slice(&last_frame);
            }
        }

        // Build the input tensor [1, N, 3, H, W] in f32 [0, 1] on device.
        // tch::Tensor expects contiguous data; we copy from u8 → f32 via
        // a temporary to avoid an in-place cast.
        let mut as_f32: Vec<f32> = Vec::with_capacity(chunk_bytes);
        for &b in chunk_buf.iter() {
            as_f32.push(b as f32 / 255.0);
        }
        // Source layout: NHWC per frame (raw RGB stride). We reshape to
        // [N, H, W, 3] then permute to [N, 3, H, W] then add batch dim.
        let input = Tensor::from_slice(&as_f32)
            .reshape([n, h, w, 3])
            .permute([0, 3, 1, 2])
            .unsqueeze(0)
            .to_device(device);

        // Forward pass — RealBasicVSR returns [1, N, 3, H*scale, W*scale].
        let output = module.forward_ts(&[input])
            .map_err(|e| anyhow::anyhow!("forward_ts: {e}"))?
            .clamp(0.0, 1.0)
            .mul(255.0);
        // Permute back to [N, H', W', 3] u8 for byte-stream output.
        let out_u8 = output
            .squeeze_dim(0)
            .permute([0, 2, 3, 1])
            .to_kind(Kind::Uint8)
            .to_device(Device::Cpu)
            .contiguous();

        let out_h = (args.height * args.scale) as i64;
        let out_w = (args.width  * args.scale) as i64;
        let per_frame = (out_h * out_w * 3) as usize;
        let total_bytes = per_frame * args.window as usize;
        let mut buf = vec![0u8; total_bytes];
        out_u8.copy_data_u8(&mut buf, total_bytes);

        // Emit only the `filled` real frames (skip the pad tail).
        for i in 0..filled {
            let off = i * per_frame;
            stdout.write_all(&buf[off..off + per_frame])?;
            emitted += 1;
            emit_progress(emitted);
        }
        stdout.flush().ok();

        if filled < args.window as usize { break; }
    }
    Ok(())
}
