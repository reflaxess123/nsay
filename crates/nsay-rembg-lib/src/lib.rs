// Shared rembg sidecar pipeline. Backend bins (nsay-rembg-cpu / -cuda /
// -dml) parse no args themselves — they call `run(setup)` with a closure
// that registers their execution provider on the SessionBuilder. The
// closure body is the one part that actually differs between backends.
//
// Protocol (mirrors crates/README.md):
//   args:   --model <onnx_path> --input <path> --output <png_path> [--choke F]
//   stderr: progress lines `stage=<name> pct=<0..100>` and human errors
//   stdout: empty on success
//   exit:   0 on success, 1 on failure
//
// Pipeline:
//   1. Load BRIA-RMBG 1.4 ONNX (single input [1,3,1024,1024], six outputs;
//      we use d1 = the first / highest-resolution mask).
//   2. Read input image (any format `image` crate handles).
//   3. Resize → 1024×1024 (bilinear), normalize per-channel:
//        x = (pixel/255 - 0.5) / 1.0
//      That matches MyPipe.preprocess in remove_bg.py.
//   4. Run inference (whatever EP the bin registered).
//   5. Resize mask back to source dimensions (bilinear).
//   6. Min-max normalize mask → u8 alpha.
//   7. Optional binary erosion (`--choke F`, 0..1 → iterations = max(1, 10*F)).
//   8. Apply alpha to the source RGB and save as PNG.

use std::io::Write;
use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use image::imageops::FilterType;
use image::{GenericImageView, ImageBuffer, Luma, Rgba, RgbaImage};
use ndarray::{Array, Array4};

// Re-export ort so backend bins don't have to declare their own pinned
// version — they get it transitively and feature-unify via cargo (cuda/
// directml feature flags on the bin propagate to this lib's ort dep).
pub use ort;
use ort::session::builder::SessionBuilder;
use ort::session::Session;
use ort::value::Tensor;

const MODEL_SIDE: u32 = 1024;

pub struct Args {
    pub model: PathBuf,
    pub input: PathBuf,
    pub output: PathBuf,
    pub choke: f32,
}

pub fn parse_args() -> Result<Args> {
    let mut model: Option<PathBuf> = None;
    let mut input: Option<PathBuf> = None;
    let mut output: Option<PathBuf> = None;
    let mut choke: f32 = 0.0;

    let mut iter = std::env::args().skip(1);
    while let Some(a) = iter.next() {
        match a.as_str() {
            "--model" => model = Some(PathBuf::from(iter.next().context("--model needs a value")?)),
            "--input" => input = Some(PathBuf::from(iter.next().context("--input needs a value")?)),
            "--output" => output = Some(PathBuf::from(iter.next().context("--output needs a value")?)),
            "--choke" => {
                choke = iter
                    .next()
                    .context("--choke needs a value")?
                    .parse()
                    .context("--choke must be a float")?;
            }
            other => bail!("unknown argument: {}", other),
        }
    }

    Ok(Args {
        model: model.context("--model is required")?,
        input: input.context("--input is required")?,
        output: output.context("--output is required")?,
        choke: choke.clamp(0.0, 1.0),
    })
}

pub fn progress(stage: &str, pct: u8) {
    // Piped stderr is *block*-buffered (line buffering only when attached
    // to a terminal). Without flush() the parent only sees a burst of
    // progress lines after the long inference step has already returned —
    // making the UI's progress bar useless.
    let mut err = std::io::stderr().lock();
    let _ = writeln!(err, "stage={} pct={}", stage, pct);
    let _ = err.flush();
}

/// Run the full rembg pipeline. `provider_setup` is called on the
/// `SessionBuilder` to register an execution provider before the model
/// is committed; CPU sidecars pass `Ok`, GPU sidecars register CUDA /
/// DirectML EPs.
pub fn run<F>(provider_setup: F) -> Result<()>
where
    F: FnOnce(SessionBuilder) -> ort::Result<SessionBuilder>,
{
    let args = parse_args()?;
    if !args.model.exists() {
        bail!("model not found: {}", args.model.display());
    }
    if !args.input.exists() {
        bail!("input not found: {}", args.input.display());
    }

    progress("load", 5);
    // commit_from_file is gated behind a feature in rc.10; reading the
    // file ourselves and using commit_from_memory works on the default
    // feature set and avoids the extra dep.
    let model_bytes = std::fs::read(&args.model)
        .with_context(|| format!("failed to read ONNX bytes from {}", args.model.display()))?;
    let builder = Session::builder()
        .map_err(|e| anyhow::anyhow!("ort session builder: {e}"))?;
    let builder = provider_setup(builder)
        .map_err(|e| anyhow::anyhow!("with_execution_providers: {e}"))?;
    let mut session = builder
        .commit_from_memory(&model_bytes)
        .map_err(|e| anyhow::anyhow!("failed to load ONNX from {}: {e}", args.model.display()))?;

    progress("read", 12);
    let img = image::open(&args.input)
        .with_context(|| format!("failed to read image {}", args.input.display()))?;
    let (orig_w, orig_h) = img.dimensions();

    progress("preprocess", 25);
    let resized = img.resize_exact(MODEL_SIDE, MODEL_SIDE, FilterType::Triangle);
    let rgb = resized.to_rgb8();
    let mut input_arr = Array4::<f32>::zeros((1, 3, MODEL_SIDE as usize, MODEL_SIDE as usize));
    for (x, y, pixel) in rgb.enumerate_pixels() {
        let xi = x as usize;
        let yi = y as usize;
        for c in 0..3 {
            let v = pixel[c] as f32 / 255.0 - 0.5;
            input_arr[[0, c, yi, xi]] = v;
        }
    }

    progress("infer", 45);
    let input_name = session
        .inputs
        .first()
        .map(|i| i.name.clone())
        .unwrap_or_else(|| "input".to_string());
    let input_tensor = Tensor::from_array(input_arr)
        .map_err(|e| anyhow::anyhow!("Tensor::from_array: {e}"))?;
    let outputs = session
        .run(ort::inputs![input_name => input_tensor])
        .map_err(|e| anyhow::anyhow!("session run: {e}"))?;

    let (_first_name, first_value) = outputs
        .iter()
        .next()
        .context("session produced no outputs")?;
    let (shape, mask_data) = first_value
        .try_extract_tensor::<f32>()
        .map_err(|e| anyhow::anyhow!("failed to extract first output as f32: {e}"))?;

    if shape.iter().any(|d| *d <= 0) {
        bail!("output has non-positive dim: {:?}", shape);
    }
    let total: usize = shape.iter().map(|d| *d as usize).product();
    if total != mask_data.len() {
        bail!(
            "output size mismatch: shape={:?} prod={} but data.len={}",
            shape, total, mask_data.len()
        );
    }
    let h = shape[shape.len() - 2] as usize;
    let w = shape[shape.len() - 1] as usize;

    progress("postprocess", 75);
    let (mut mn, mut mx) = (f32::INFINITY, f32::NEG_INFINITY);
    for &v in mask_data.iter() {
        if v < mn { mn = v; }
        if v > mx { mx = v; }
    }
    let span = (mx - mn).max(1e-6);

    let mask_view = Array::from_shape_vec((h, w), mask_data.to_vec())?;
    let mut mask_u8: ImageBuffer<Luma<u8>, Vec<u8>> = ImageBuffer::new(w as u32, h as u32);
    for ((y, x), v) in mask_view.indexed_iter() {
        let n = ((*v - mn) / span).clamp(0.0, 1.0);
        mask_u8.put_pixel(x as u32, y as u32, Luma([(n * 255.0) as u8]));
    }

    let mask_full = image::imageops::resize(&mask_u8, orig_w, orig_h, FilterType::Triangle);

    let final_mask = if args.choke > 0.0 {
        let iterations = ((args.choke * 10.0).round() as u8).max(1);
        let bin = imageproc::contrast::threshold(
            &mask_full, 128, imageproc::contrast::ThresholdType::Binary,
        );
        let eroded = imageproc::morphology::erode(
            &bin, imageproc::distance_transform::Norm::LInf, iterations,
        );
        let mut soft_eroded: ImageBuffer<Luma<u8>, Vec<u8>> = ImageBuffer::new(orig_w, orig_h);
        for (x, y, soft) in mask_full.enumerate_pixels() {
            let keep = eroded.get_pixel(x, y)[0];
            let v = if keep > 0 { soft[0] } else { 0 };
            soft_eroded.put_pixel(x, y, Luma([v]));
        }
        soft_eroded
    } else {
        mask_full
    };

    progress("save", 90);
    let src_rgba = img.to_rgba8();
    let mut out: RgbaImage = ImageBuffer::new(orig_w, orig_h);
    for (x, y, pixel) in src_rgba.enumerate_pixels() {
        let alpha = final_mask.get_pixel(x, y)[0];
        out.put_pixel(x, y, Rgba([pixel[0], pixel[1], pixel[2], alpha]));
    }
    if let Some(parent) = args.output.parent() {
        std::fs::create_dir_all(parent).ok();
    }
    out.save(&args.output)
        .with_context(|| format!("failed to write {}", args.output.display()))?;

    progress("done", 100);
    Ok(())
}
