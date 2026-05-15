// Shared rembg sidecar pipeline. Backend bins (nsay-rembg-cpu / -cuda /
// -dml) call `run(setup)` with a closure that registers their execution
// provider on the SessionBuilder. The closure body is the one part that
// actually differs between backends.
//
// Protocol:
//   args:   --model <onnx_path> --input <path> --output <png_path>
//           [--preset bria-rmbg|ben2] [--choke F]
//   stderr: progress lines `stage=<name> pct=<0..100>` and human errors
//   stdout: empty on success
//   exit:   0 on success, 1 on failure
//
// Pipeline (per Preset; default = bria-rmbg for back-compat):
//   1. Load ONNX into the registered EP's session.
//   2. Read input image.
//   3. Resize to preset.input_size() with preset.resize_filter().
//   4. Normalize per-channel: x = (px/255 - mean) / std (preset-specific).
//   5. session.run() → take first output.
//   6. If preset.needs_sigmoid() → apply 1/(1+exp(-x)) elementwise.
//   7. Min-max normalize to [0,1] → uint8 alpha mask at model resolution.
//   8. Resize alpha back to source dims (bilinear).
//   9. Optional binary erosion (`--choke F`).
//  10. Apply alpha to source RGB and save as PNG.

use std::io::Write;
use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use image::imageops::FilterType;
use image::{GenericImageView, ImageBuffer, Luma, Rgba, RgbaImage};
use ndarray::{Array, Array4};

pub use ort;
use ort::session::builder::SessionBuilder;
use ort::session::Session;
use ort::value::Tensor;

/// Per-model preprocessing/postprocessing flavour. Catalog entry's
/// `preset` field is forwarded to the sidecar via `--preset`. Adding a
/// new model = add a variant + match arms; bins don't change.
#[derive(Debug, Clone, Copy)]
pub enum Preset {
    /// BRIA RMBG 1.4 — original. 1024², Triangle resize,
    /// `(px/255 - 0.5)` normalize, no sigmoid (model already saturates).
    BriaRmbg,
    /// BEN2 (PramaLLC/BEN2). 1024², Lanczos resize, ImageNet normalize,
    /// sigmoid baked into the ONNX export → no extra sigmoid in lib.
    Ben2,
}

impl Preset {
    fn parse(s: &str) -> Result<Self> {
        match s {
            "bria-rmbg" => Ok(Self::BriaRmbg),
            "ben2"      => Ok(Self::Ben2),
            other => bail!("unknown rembg preset: {} (expected bria-rmbg | ben2)", other),
        }
    }

    fn input_size(&self) -> u32 {
        match self {
            Self::BriaRmbg | Self::Ben2 => 1024,
        }
    }

    fn resize_filter(&self) -> FilterType {
        match self {
            Self::BriaRmbg => FilterType::Triangle,
            Self::Ben2     => FilterType::Lanczos3,
        }
    }

    /// Per-channel (mean, std). Input pixel formula: `(px/255 - mean) / std`.
    fn normalize(&self) -> ([f32; 3], [f32; 3]) {
        match self {
            Self::BriaRmbg => ([0.5, 0.5, 0.5], [1.0, 1.0, 1.0]),
            Self::Ben2     => ([0.485, 0.456, 0.406], [0.229, 0.224, 0.225]),
        }
    }

    /// Whether to apply sigmoid to raw model output before min-max.
    /// BRIA RMBG: outputs already saturate to near-binary, min-max is enough.
    /// BEN2: sigmoid is baked into the ONNX export → don't double-apply.
    fn needs_sigmoid(&self) -> bool {
        match self {
            Self::BriaRmbg | Self::Ben2 => false,
        }
    }
}

pub struct Args {
    pub model: PathBuf,
    pub input: PathBuf,
    pub output: PathBuf,
    pub preset: Preset,
    pub choke: f32,
}

pub fn parse_args() -> Result<Args> {
    let mut model: Option<PathBuf> = None;
    let mut input: Option<PathBuf> = None;
    let mut output: Option<PathBuf> = None;
    let mut preset_str: String = "bria-rmbg".to_string();
    let mut choke: f32 = 0.0;

    let mut iter = std::env::args().skip(1);
    while let Some(a) = iter.next() {
        match a.as_str() {
            "--model"  => model  = Some(PathBuf::from(iter.next().context("--model needs a value")?)),
            "--input"  => input  = Some(PathBuf::from(iter.next().context("--input needs a value")?)),
            "--output" => output = Some(PathBuf::from(iter.next().context("--output needs a value")?)),
            "--preset" => preset_str = iter.next().context("--preset needs a value")?,
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
        preset: Preset::parse(&preset_str)?,
        choke: choke.clamp(0.0, 1.0),
    })
}

pub fn progress(stage: &str, pct: u8) {
    let mut err = std::io::stderr().lock();
    let _ = writeln!(err, "stage={} pct={}", stage, pct);
    let _ = err.flush();
}

/// Run the full rembg pipeline. `provider_setup` registers the EP — CPU
/// passes `Ok`, GPU bins build a CUDA / DirectML provider.
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

    let preset = args.preset;
    let model_side = preset.input_size();
    let resize_filter = preset.resize_filter();
    let (mean, std) = preset.normalize();

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

    progress("read", 12);
    let img = image::open(&args.input)
        .with_context(|| format!("failed to read image {}", args.input.display()))?;
    let (orig_w, orig_h) = img.dimensions();

    progress("preprocess", 25);
    let resized = img.resize_exact(model_side, model_side, resize_filter);
    let rgb = resized.to_rgb8();
    let mut input_arr = Array4::<f32>::zeros((1, 3, model_side as usize, model_side as usize));
    for (x, y, pixel) in rgb.enumerate_pixels() {
        let xi = x as usize;
        let yi = y as usize;
        for c in 0..3 {
            let v = (pixel[c] as f32 / 255.0 - mean[c]) / std[c];
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
    let (shape, raw_data) = first_value
        .try_extract_tensor::<f32>()
        .map_err(|e| anyhow::anyhow!("failed to extract first output as f32: {e}"))?;

    if shape.iter().any(|d| *d <= 0) {
        bail!("output has non-positive dim: {:?}", shape);
    }
    let total: usize = shape.iter().map(|d| *d as usize).product();
    if total != raw_data.len() {
        bail!(
            "output size mismatch: shape={:?} prod={} but data.len={}",
            shape, total, raw_data.len()
        );
    }
    let h = shape[shape.len() - 2] as usize;
    let w = shape[shape.len() - 1] as usize;

    // Optional sigmoid for presets that export raw logits (BiRefNet,
    // future). BEN2 / BRIA RMBG are pre-saturated so we skip.
    let mask_data: Vec<f32> = if preset.needs_sigmoid() {
        raw_data.iter().map(|&v| 1.0 / (1.0 + (-v).exp())).collect()
    } else {
        raw_data.to_vec()
    };

    progress("postprocess", 75);
    let (mut mn, mut mx) = (f32::INFINITY, f32::NEG_INFINITY);
    for &v in mask_data.iter() {
        if v < mn { mn = v; }
        if v > mx { mx = v; }
    }
    let span = (mx - mn).max(1e-6);

    let mask_view = Array::from_shape_vec((h, w), mask_data)?;
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
