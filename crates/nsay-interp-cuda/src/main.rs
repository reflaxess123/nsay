// nsay-interp-cuda — CUDA frame interpolation sidecar (RIFE 4.9).
//
// Streaming-only protocol (no file mode — interp is always video):
//   args:   --model <onnx_path> --stream --width W --height H --factor K
//   stdin:  raw RGB frames, W*H*3 bytes per frame
//   stdout: raw RGB frames, W*H*3 bytes per frame, total = 1 + (N-1)*K
//   stderr: progress lines `frame N` after each emitted output frame
//   exit:   0 success, 1 failure
//
// Pipeline (per pair of input frames a, b):
//   for k in 1..K:
//     t = k / K
//     interp = RIFE(a, b, t)        ; emit interp                (K-1 frames)
//   emit b                          ;                            (1 frame)
//
// Plus a single emit of the very first input frame before the loop.
// Total output for N inputs at factor K: 1 + (N-1)*K.
//
// RIFE input dims must be padded to multiples of 32 (or 64 for some
// exports). We reflect-pad on input, crop on output. The yuvraj108c
// ensemble export accepts any size that's a multiple of 32; we round up.

use std::io::{Read, Write};
use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use ndarray::{Array1, Array4};
use ort::execution_providers::CUDAExecutionProvider;
use ort::session::Session;
use ort::value::Tensor;

const ALIGN: u32 = 32;

struct Args {
    model: PathBuf,
    width: u32,
    height: u32,
    factor: u32,
}

fn parse_args() -> Result<Args> {
    let mut model: Option<PathBuf> = None;
    let mut width: u32 = 0;
    let mut height: u32 = 0;
    let mut factor: u32 = 2;
    let mut stream = false;

    let mut iter = std::env::args().skip(1);
    while let Some(a) = iter.next() {
        match a.as_str() {
            "--model"  => model  = Some(PathBuf::from(iter.next().context("--model needs a value")?)),
            "--width"  => width  = iter.next().context("--width needs a value")?.parse()?,
            "--height" => height = iter.next().context("--height needs a value")?.parse()?,
            "--factor" => factor = iter.next().context("--factor needs a value")?.parse()?,
            "--stream" => stream = true,
            other => bail!("unknown argument: {}", other),
        }
    }
    if !stream { bail!("--stream is required (interp is video-only)"); }
    if width == 0 || height == 0 { bail!("--width and --height required"); }
    if !(2..=16).contains(&factor) { bail!("--factor must be 2..=16, got {}", factor); }

    Ok(Args { model: model.context("--model is required")?, width, height, factor })
}

fn main() {
    if let Err(e) = run() {
        let _ = writeln!(std::io::stderr(), "nsay-interp-cuda error: {e:#}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let args = parse_args()?;
    if !args.model.exists() { bail!("model not found: {}", args.model.display()); }

    let model_bytes = std::fs::read(&args.model)?;
    let mut session = build_session(&model_bytes)?;

    // Discover input names — RIFE exports vary across uploaders.
    // yuvraj108c/rife-onnx ensemble: img0, img1, timestep
    let input_names: Vec<String> = session.inputs.iter().map(|i| i.name.clone()).collect();
    if input_names.len() < 3 {
        bail!("expected ≥3 inputs (img0, img1, timestep), got {:?}", input_names);
    }
    let n_img0 = input_names[0].clone();
    let n_img1 = input_names[1].clone();
    let n_t    = input_names[2].clone();

    // Padded dims for the model — round up to ALIGN. We pad with edge
    // replication so the optical-flow estimator doesn't see hard zero
    // borders that would warp the output near the seams.
    let pad_w = ((args.width  + ALIGN - 1) / ALIGN) * ALIGN;
    let pad_h = ((args.height + ALIGN - 1) / ALIGN) * ALIGN;

    let _ = writeln!(std::io::stderr(), "stream-ready");
    let _ = std::io::stderr().flush();

    let frame_bytes = (args.width as usize) * (args.height as usize) * 3;
    let mut stdin = std::io::stdin().lock();
    let mut stdout = std::io::stdout().lock();

    // Read first frame.
    let mut buf_a = vec![0u8; frame_bytes];
    if stdin.read_exact(&mut buf_a).is_err() {
        // No input at all — exit cleanly.
        return Ok(());
    }
    // Emit it as the first output frame.
    stdout.write_all(&buf_a)?;
    stdout.flush().ok();
    let mut emitted: u64 = 1;
    emit_progress(emitted);

    let mut buf_b = vec![0u8; frame_bytes];
    loop {
        match stdin.read_exact(&mut buf_b) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(e) => return Err(anyhow::anyhow!("stdin read failed: {e}")),
        }
        let img_a = pad_to_chw(&buf_a, args.width, args.height, pad_w, pad_h);
        let img_b = pad_to_chw(&buf_b, args.width, args.height, pad_w, pad_h);

        for k in 1..args.factor {
            let t = k as f32 / args.factor as f32;
            // yuvraj108c/rife-onnx ensemble: timestep is rank-1 length-1
            // (a scalar wrapped in a 1D tensor). NOT a broadcast HxW mask.
            let timestep = Array1::<f32>::from_elem(1, t);
            let interp = run_rife(
                &mut session, &n_img0, &n_img1, &n_t,
                img_a.clone(), img_b.clone(), timestep,
            )?;
            let cropped = crop_chw_to_rgb(&interp, pad_w, pad_h, args.width, args.height);
            stdout.write_all(&cropped)?;
            stdout.flush().ok();
            emitted += 1;
            emit_progress(emitted);
        }
        // Emit b as the last frame of this segment; it doubles as the
        // start of the next pair without re-running RIFE.
        stdout.write_all(&buf_b)?;
        stdout.flush().ok();
        emitted += 1;
        emit_progress(emitted);

        std::mem::swap(&mut buf_a, &mut buf_b);
    }
    Ok(())
}

fn emit_progress(n: u64) {
    let _ = writeln!(std::io::stderr(), "frame {}", n);
    let _ = std::io::stderr().flush();
}

fn build_session(model_bytes: &[u8]) -> Result<Session> {
    Session::builder()
        .map_err(|e| anyhow::anyhow!("ort builder: {e}"))?
        .with_execution_providers([CUDAExecutionProvider::default().build().error_on_failure()])
        .map_err(|e| anyhow::anyhow!("with_execution_providers (cuda): {e}"))?
        .commit_from_memory(model_bytes)
        .map_err(|e| anyhow::anyhow!("commit_from_memory: {e}"))
}

/// Reflect-pad a raw RGB frame to (pad_w, pad_h) and convert to CHW
/// f32 normalized to [0,1]. Edge replication on overhang.
fn pad_to_chw(rgb: &[u8], src_w: u32, src_h: u32, pad_w: u32, pad_h: u32) -> Array4<f32> {
    let mut arr = Array4::<f32>::zeros((1, 3, pad_h as usize, pad_w as usize));
    for y in 0..pad_h {
        let sy = y.min(src_h - 1) as usize;
        for x in 0..pad_w {
            let sx = x.min(src_w - 1) as usize;
            let i = (sy * src_w as usize + sx) * 3;
            arr[[0, 0, y as usize, x as usize]] = rgb[i]     as f32 / 255.0;
            arr[[0, 1, y as usize, x as usize]] = rgb[i + 1] as f32 / 255.0;
            arr[[0, 2, y as usize, x as usize]] = rgb[i + 2] as f32 / 255.0;
        }
    }
    arr
}

/// Crop a CHW f32 [3, pad_h, pad_w] back to raw RGB at (src_w, src_h).
fn crop_chw_to_rgb(chw: &[f32], pad_w: u32, pad_h: u32, src_w: u32, src_h: u32) -> Vec<u8> {
    let mut out = vec![0u8; (src_w * src_h * 3) as usize];
    let plane = (pad_w * pad_h) as usize;
    for y in 0..src_h {
        for x in 0..src_w {
            let src_i = (y * pad_w + x) as usize;
            let dst_i = ((y * src_w + x) * 3) as usize;
            out[dst_i]     = (chw[src_i].clamp(0.0, 1.0) * 255.0) as u8;
            out[dst_i + 1] = (chw[plane + src_i].clamp(0.0, 1.0) * 255.0) as u8;
            out[dst_i + 2] = (chw[2 * plane + src_i].clamp(0.0, 1.0) * 255.0) as u8;
        }
    }
    out
}

fn run_rife(
    session: &mut Session,
    n_img0: &str, n_img1: &str, n_t: &str,
    img0: Array4<f32>, img1: Array4<f32>, timestep: Array1<f32>,
) -> Result<Vec<f32>> {
    let t0 = Tensor::from_array(img0).map_err(|e| anyhow::anyhow!("tensor img0: {e}"))?;
    let t1 = Tensor::from_array(img1).map_err(|e| anyhow::anyhow!("tensor img1: {e}"))?;
    let tt = Tensor::from_array(timestep).map_err(|e| anyhow::anyhow!("tensor t: {e}"))?;
    let outputs = session
        .run(ort::inputs![n_img0 => t0, n_img1 => t1, n_t => tt])
        .map_err(|e| anyhow::anyhow!("session run: {e}"))?;
    let (_, first) = outputs.iter().next().context("session produced no outputs")?;
    let (_shape, data) = first.try_extract_tensor::<f32>()
        .map_err(|e| anyhow::anyhow!("extract output: {e}"))?;
    Ok(data.to_vec())
}
