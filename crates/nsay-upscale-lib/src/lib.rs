// Shared upscale sidecar pipeline. Backend bins (nsay-upscale-cpu /
// -cuda / -dml) call `run(setup)` with a closure that registers their
// execution provider on the SessionBuilder. Everything else — arg parsing,
// tile blending, streaming, image I/O — lives here.
//
// Protocol:
//   File mode:
//     args:   --model <onnx_path> --input <path> --output <png_path>
//             [--scale F] [--model-scale N]
//   Stream mode (used by the video runner):
//     args:   --model <onnx_path> --stream --width W --height H
//             [--scale F] [--model-scale N]
//     stdin:  raw RGB frames, W*H*3 bytes per frame
//     stdout: raw RGB frames, W*scale*H*scale*3 bytes per frame
//   stderr: file mode → `stage=<name> pct=<0..100>`;
//           stream mode → `stream-ready`, then `frame N` per output frame
//   exit:   0 success, 1 failure
//
// Pipeline (per frame):
//   1. If user --scale != model native (`--model-scale`), resize source by
//      ratio = scale/model_scale before inference. The model still runs at
//      its native ratio, so total scaling = scale.
//   2. If both dims ≤ SINGLE_PASS_LIMIT → one inference pass on the whole
//      image. Otherwise tile with TILE_SIZE × TILE_SIZE chunks and Hann-
//      window blend the results to hide seams.

use std::io::{Read, Write};
use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use image::imageops::FilterType;
use image::{DynamicImage, GenericImageView, ImageBuffer, Rgb, RgbImage};
use ndarray::Array4;

pub use ort;
use ort::session::builder::SessionBuilder;
use ort::session::Session;
use ort::value::Tensor;

/// If both source dimensions are ≤ this many pixels, we run one inference
/// pass on the whole image (no tiling). 1024² float input × 4 (output 4×)
/// = ~50 MB VRAM working set, fits everywhere.
const SINGLE_PASS_LIMIT: u32 = 1024;
/// Tile size in input pixels for sources larger than SINGLE_PASS_LIMIT.
/// 512² × f32 × 3 = 3 MB input, 12 MB output per tile. Comfortable for
/// any consumer GPU and big enough to amortise per-call overhead.
const TILE_SIZE: u32 = 512;
/// Overlap between adjacent tiles (input pixels). The Hann window blend
/// removes the visible seam this overlap creates.
const OVERLAP: u32 = 32;

pub struct Args {
    pub model: PathBuf,
    pub input: Option<PathBuf>,
    pub output: Option<PathBuf>,
    /// Final output scale relative to source: 1.5, 2, 3, or 4.
    pub scale: f32,
    /// Native output:input ratio of the loaded model (2 or 4).
    pub model_scale: u32,
    /// Streaming mode: skip image::open / save, instead read raw RGB frames
    /// from stdin (width*height*3 bytes each) and write upscaled raw RGB to
    /// stdout. Used by the video runner. Requires --width and --height.
    pub stream: bool,
    pub width: u32,
    pub height: u32,
}

pub fn parse_args() -> Result<Args> {
    let mut model: Option<PathBuf> = None;
    let mut input: Option<PathBuf> = None;
    let mut output: Option<PathBuf> = None;
    let mut scale: f32 = 4.0;
    let mut model_scale: u32 = 4;
    let mut stream = false;
    let mut width: u32 = 0;
    let mut height: u32 = 0;

    let mut iter = std::env::args().skip(1);
    while let Some(a) = iter.next() {
        match a.as_str() {
            "--model" => model = Some(PathBuf::from(iter.next().context("--model needs a value")?)),
            "--input" => input = Some(PathBuf::from(iter.next().context("--input needs a value")?)),
            "--output" => output = Some(PathBuf::from(iter.next().context("--output needs a value")?)),
            "--scale" => {
                scale = iter.next().context("--scale needs a value")?
                    .parse().context("--scale must be a number in [1.0, 4.0]")?;
            }
            "--model-scale" => {
                model_scale = iter.next().context("--model-scale needs a value")?
                    .parse().context("--model-scale must be 2 or 4")?;
            }
            "--stream" => stream = true,
            "--width"  => width  = iter.next().context("--width needs a value")?.parse()?,
            "--height" => height = iter.next().context("--height needs a value")?.parse()?,
            // Accept and ignore --choke for protocol uniformity with rembg.
            "--choke" => { let _ = iter.next(); }
            other => bail!("unknown argument: {}", other),
        }
    }
    if !(1.0..=4.0).contains(&scale) {
        bail!("--scale must be in [1.0, 4.0] (got {})", scale);
    }
    if !(2..=4).contains(&model_scale) {
        bail!("--model-scale must be 2, 3, or 4 (got {})", model_scale);
    }
    if stream {
        if width == 0 || height == 0 {
            bail!("--stream requires --width and --height");
        }
    } else if input.is_none() || output.is_none() {
        bail!("--input and --output required in file mode (or pass --stream)");
    }

    Ok(Args {
        model: model.context("--model is required")?,
        input, output, scale, model_scale, stream, width, height,
    })
}

pub fn progress(stage: &str, pct: u8) {
    let mut err = std::io::stderr().lock();
    let _ = writeln!(err, "stage={} pct={}", stage, pct);
    let _ = err.flush();
}

/// Run the full upscale sidecar (file or stream mode based on args).
/// `provider_setup` registers the EP — CPU passes `Ok`, GPU bins build a
/// CUDA / DirectML provider.
pub fn run<F>(provider_setup: F) -> Result<()>
where
    F: FnOnce(SessionBuilder) -> ort::Result<SessionBuilder>,
{
    let args = parse_args()?;
    if !args.model.exists() {
        bail!("model not found: {}", args.model.display());
    }

    progress("load", 5);
    let model_bytes = std::fs::read(&args.model)
        .with_context(|| format!("failed to read ONNX bytes from {}", args.model.display()))?;
    let builder = Session::builder()
        .map_err(|e| anyhow::anyhow!("ort session builder: {e}"))?;
    let builder = provider_setup(builder)
        .map_err(|e| anyhow::anyhow!("with_execution_providers: {e}"))?;
    let mut session = builder
        .commit_from_memory(&model_bytes)
        .map_err(|e| anyhow::anyhow!("failed to load ONNX from {}: {e}", args.model.display()))?;
    let input_name = session.inputs.first().map(|i| i.name.clone()).unwrap_or_else(|| "input".to_string());

    if args.stream {
        run_stream(&mut session, &input_name, &args)
    } else {
        run_file(&mut session, &input_name, &args)
    }
}

fn run_file(session: &mut Session, input_name: &str, args: &Args) -> Result<()> {
    let input_path = args.input.as_ref().expect("file mode");
    let output_path = args.output.as_ref().expect("file mode");
    if !input_path.exists() { bail!("input not found: {}", input_path.display()); }

    progress("read", 10);
    let img = image::open(input_path)
        .with_context(|| format!("failed to read image {}", input_path.display()))?;
    let (src_w, src_h) = img.dimensions();
    let rgb_src = img.to_rgb8();

    progress("infer", 15);
    let out_img = process_frame(session, input_name, &rgb_src, src_w, src_h, args.scale, args.model_scale)?;
    progress("infer", 85);

    progress("save", 96);
    if let Some(parent) = output_path.parent() { std::fs::create_dir_all(parent).ok(); }
    out_img.save(output_path)
        .with_context(|| format!("failed to write {}", output_path.display()))?;
    progress("done", 100);
    Ok(())
}

fn run_stream(session: &mut Session, input_name: &str, args: &Args) -> Result<()> {
    let frame_in_bytes = (args.width as usize) * (args.height as usize) * 3;
    let max_out_pixels: u64 = 4 * 1024 * 1024 * 1024 / 12;
    let est_out_pixels = (args.width as u64) * (args.height as u64)
        * ((args.scale * args.scale) as u64);
    if est_out_pixels > max_out_pixels {
        bail!("frame too large for streaming x{:.1}: {}x{}", args.scale, args.width, args.height);
    }

    let _ = writeln!(std::io::stderr(), "stream-ready");
    let _ = std::io::stderr().flush();

    let mut stdin = std::io::stdin().lock();
    let mut stdout = std::io::stdout().lock();
    let mut buf = vec![0u8; frame_in_bytes];
    let mut frame_idx: u64 = 0;

    loop {
        match stdin.read_exact(&mut buf) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(e) => return Err(anyhow::anyhow!("stdin read failed at frame {}: {e}", frame_idx)),
        }
        let rgb_in: RgbImage = ImageBuffer::from_raw(args.width, args.height, buf.clone())
            .ok_or_else(|| anyhow::anyhow!("malformed frame {}", frame_idx))?;
        let out_img = process_frame(session, input_name, &rgb_in, args.width, args.height, args.scale, args.model_scale)?;
        stdout.write_all(out_img.as_raw())?;
        stdout.flush().ok();
        frame_idx += 1;
        let _ = writeln!(std::io::stderr(), "frame {}", frame_idx);
        let _ = std::io::stderr().flush();
    }
    Ok(())
}

fn process_frame(
    session: &mut Session, input_name: &str, src: &RgbImage,
    src_w: u32, src_h: u32, scale: f32, model_scale: u32,
) -> Result<RgbImage> {
    let ratio = scale / model_scale as f32;
    let (in_w, in_h, rgb) = if (ratio - 1.0).abs() < f32::EPSILON {
        (src_w, src_h, src.clone())
    } else {
        let new_w = ((src_w as f32 * ratio).round() as u32).max(1);
        let new_h = ((src_h as f32 * ratio).round() as u32).max(1);
        let scaled = DynamicImage::ImageRgb8(src.clone())
            .resize_exact(new_w, new_h, FilterType::Triangle);
        (new_w, new_h, scaled.to_rgb8())
    };
    let out_w = in_w * model_scale;
    let out_h = in_h * model_scale;
    if in_w <= SINGLE_PASS_LIMIT && in_h <= SINGLE_PASS_LIMIT {
        let arr = pixels_to_chw(&rgb, 0, 0, in_w, in_h);
        let tile = run_tile(session, input_name, arr, in_w, in_h, model_scale)?;
        Ok(chw_to_image(&tile, out_w, out_h))
    } else {
        tiled_inference(session, input_name, &rgb, in_w, in_h, out_w, out_h, model_scale)
    }
}

fn pixels_to_chw(
    rgb: &image::RgbImage, ox: u32, oy: u32, w: u32, h: u32,
) -> Array4<f32> {
    let mut arr = Array4::<f32>::zeros((1, 3, h as usize, w as usize));
    for ty in 0..h {
        for tx in 0..w {
            let p = rgb.get_pixel(ox + tx, oy + ty);
            for c in 0..3 {
                arr[[0, c, ty as usize, tx as usize]] = p[c] as f32 / 255.0;
            }
        }
    }
    arr
}

fn run_tile(
    session: &mut Session,
    input_name: &str,
    arr: Array4<f32>,
    in_w: u32, in_h: u32,
    model_scale: u32,
) -> Result<Vec<f32>> {
    let input_tensor = Tensor::from_array(arr)
        .map_err(|e| anyhow::anyhow!("Tensor::from_array: {e}"))?;
    let outputs = session
        .run(ort::inputs![input_name => input_tensor])
        .map_err(|e| anyhow::anyhow!("session run ({}x{}): {e}", in_w, in_h))?;
    let (_n, first) = outputs
        .iter()
        .next()
        .context("session produced no outputs")?;
    let (shape, data) = first
        .try_extract_tensor::<f32>()
        .map_err(|e| anyhow::anyhow!("extract output: {e}"))?;
    if shape.len() < 4 {
        bail!("unexpected output rank: {:?}", shape);
    }
    let oh = shape[shape.len() - 2] as u32;
    let ow = shape[shape.len() - 1] as u32;
    if oh != in_h * model_scale || ow != in_w * model_scale {
        bail!(
            "unexpected output tile size: got {}x{}, expected {}x{}",
            ow, oh, in_w * model_scale, in_h * model_scale
        );
    }
    Ok(data.to_vec())
}

fn chw_to_image(chw: &[f32], w: u32, h: u32) -> RgbImage {
    let plane = (w * h) as usize;
    let mut out: RgbImage = ImageBuffer::new(w, h);
    for y in 0..h {
        for x in 0..w {
            let i = (y * w + x) as usize;
            let r = chw[i].clamp(0.0, 1.0);
            let g = chw[plane + i].clamp(0.0, 1.0);
            let b = chw[2 * plane + i].clamp(0.0, 1.0);
            out.put_pixel(
                x, y,
                Rgb([(r * 255.0) as u8, (g * 255.0) as u8, (b * 255.0) as u8]),
            );
        }
    }
    out
}

fn hann(i: u32, n: u32) -> f32 {
    if n <= 1 { return 1.0; }
    let t = i as f32 / (n - 1) as f32;
    0.5 * (1.0 - (std::f32::consts::TAU * t).cos())
}

fn tiled_inference(
    session: &mut Session,
    input_name: &str,
    rgb: &image::RgbImage,
    in_w: u32, in_h: u32,
    out_w: u32, out_h: u32,
    model_scale: u32,
) -> Result<RgbImage> {
    let mut out_acc: Vec<f32> = vec![0.0; (out_w * out_h * 3) as usize];
    let mut out_w_acc: Vec<f32> = vec![0.0; (out_w * out_h) as usize];

    let starts = |total: u32| -> Vec<u32> {
        if total <= TILE_SIZE { return vec![0]; }
        let step = TILE_SIZE - OVERLAP;
        let mut s = Vec::new();
        let mut x = 0u32;
        while x + TILE_SIZE < total {
            s.push(x);
            x += step;
        }
        s.push(total - TILE_SIZE);
        s
    };
    let xs = starts(in_w);
    let ys = starts(in_h);
    let total_tiles = (xs.len() * ys.len()) as f32;
    let mut done_tiles = 0f32;

    for &y0 in &ys {
        for &x0 in &xs {
            let tw = (in_w.min(x0 + TILE_SIZE) - x0).min(TILE_SIZE);
            let th = (in_h.min(y0 + TILE_SIZE) - y0).min(TILE_SIZE);
            let arr = pixels_to_chw(rgb, x0, y0, tw, th);
            let data = run_tile(session, input_name, arr, tw, th, model_scale)?;

            let ox0 = x0 * model_scale;
            let oy0 = y0 * model_scale;
            let ow = tw * model_scale;
            let oh = th * model_scale;
            let plane = (ow * oh) as usize;
            for ly in 0..oh {
                let wy = hann(ly, oh);
                for lx in 0..ow {
                    let w = wy * hann(lx, ow);
                    let canvas_x = ox0 + lx;
                    let canvas_y = oy0 + ly;
                    let canvas_idx = (canvas_y * out_w + canvas_x) as usize;
                    out_w_acc[canvas_idx] += w;
                    for c in 0..3 {
                        let src = data[c * plane + (ly * ow + lx) as usize];
                        out_acc[canvas_idx * 3 + c] += src * w;
                    }
                }
            }

            done_tiles += 1.0;
            let pct = (12.0 + (done_tiles / total_tiles) * 76.0) as u8;
            progress("infer", pct);
        }
    }

    let mut out_img: RgbImage = ImageBuffer::new(out_w, out_h);
    for y in 0..out_h {
        for x in 0..out_w {
            let canvas_idx = (y * out_w + x) as usize;
            let w = out_w_acc[canvas_idx].max(1e-6);
            let r = (out_acc[canvas_idx * 3] / w).clamp(0.0, 1.0);
            let g = (out_acc[canvas_idx * 3 + 1] / w).clamp(0.0, 1.0);
            let b = (out_acc[canvas_idx * 3 + 2] / w).clamp(0.0, 1.0);
            out_img.put_pixel(
                x, y,
                Rgb([(r * 255.0) as u8, (g * 255.0) as u8, (b * 255.0) as u8]),
            );
        }
    }
    Ok(out_img)
}
