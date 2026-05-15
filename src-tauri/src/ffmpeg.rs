// ffmpeg / ffprobe path resolution and one-time hardware-encoder detection.
//
// Bundled binaries live at <exe>/binaries/ffmpeg/{ffmpeg,ffprobe}.exe (via
// tauri.conf.json bundle.resources). In `tauri dev` the exe sits in
// target/debug/, so we also walk up to the repo's src-tauri/binaries/.
//
// Encoder detection is *cached for the lifetime of the process* — running
// `ffmpeg -encoders` is ~150 ms, expensive to repeat per video.

use anyhow::{bail, Context, Result};
use std::path::PathBuf;
use std::process::Command;
use std::sync::OnceLock;

#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;
#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

#[cfg(target_os = "windows")]
const FFMPEG_BIN: &str = "ffmpeg.exe";
#[cfg(target_os = "windows")]
const FFPROBE_BIN: &str = "ffprobe.exe";
#[cfg(not(target_os = "windows"))]
const FFMPEG_BIN: &str = "ffmpeg";
#[cfg(not(target_os = "windows"))]
const FFPROBE_BIN: &str = "ffprobe";

pub fn ffmpeg_path() -> Result<PathBuf> { resolve_bin(FFMPEG_BIN) }
pub fn ffprobe_path() -> Result<PathBuf> { resolve_bin(FFPROBE_BIN) }

fn resolve_bin(name: &str) -> Result<PathBuf> {
    // 1. dev override
    if let Ok(env_dir) = std::env::var("NSAY_FFMPEG_DIR") {
        let p = PathBuf::from(env_dir).join(name);
        if p.exists() { return Ok(p); }
    }
    let exe = std::env::current_exe().context("current_exe failed")?;
    let exe_dir = exe.parent().context("exe has no parent")?;

    // 2. tauri bundle layout
    let p = exe_dir.join("binaries").join("ffmpeg").join(name);
    if p.exists() { return Ok(p); }
    // 3. flat-resource fallback
    let p = exe_dir.join(name);
    if p.exists() { return Ok(p); }

    // 4. dev: walk up to find src-tauri/binaries/ffmpeg/
    let mut dir: &std::path::Path = exe_dir;
    loop {
        let cand = dir.join("src-tauri").join("binaries").join("ffmpeg").join(name);
        if cand.exists() { return Ok(cand); }
        match dir.parent() { Some(p) => dir = p, None => break }
    }
    bail!("could not locate {} — tried NSAY_FFMPEG_DIR, exe-relative, and repo src-tauri/binaries/ffmpeg/", name)
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct EncoderChoice {
    /// h264 encoder name, e.g. "libx264" / "h264_nvenc"
    pub h264: &'static str,
    /// hevc encoder name, e.g. "libx265" / "hevc_nvenc"
    pub hevc: &'static str,
    /// human family label for the UI: "nvenc" | "qsv" | "amf" | "x264"
    pub family: &'static str,
}

static ENCODER: OnceLock<EncoderChoice> = OnceLock::new();

/// Detect (and cache) the best available video encoder by querying
/// `ffmpeg -encoders` once. Priority: NVENC > QSV > AMF > libx264.
/// Falls back to libx264 on any failure (no panics — the video runner can
/// always fall back to software).
pub fn detect_encoder() -> EncoderChoice {
    ENCODER.get_or_init(|| {
        let raw = run_ffmpeg_encoders().unwrap_or_default();
        // Each branch checks h264_<vendor> first because that's the more
        // commonly used codec; if the vendor has h264 they also have hevc.
        if raw.contains(" h264_nvenc ") {
            return EncoderChoice {
                h264: "h264_nvenc",
                hevc: if raw.contains(" hevc_nvenc ") { "hevc_nvenc" } else { "libx265" },
                family: "nvenc",
            };
        }
        if raw.contains(" h264_qsv ") {
            return EncoderChoice {
                h264: "h264_qsv",
                hevc: if raw.contains(" hevc_qsv ") { "hevc_qsv" } else { "libx265" },
                family: "qsv",
            };
        }
        if raw.contains(" h264_amf ") {
            return EncoderChoice {
                h264: "h264_amf",
                hevc: if raw.contains(" hevc_amf ") { "hevc_amf" } else { "libx265" },
                family: "amf",
            };
        }
        EncoderChoice { h264: "libx264", hevc: "libx265", family: "x264" }
    }).clone()
}

fn run_ffmpeg_encoders() -> Result<String> {
    let bin = ffmpeg_path()?;
    let mut cmd = Command::new(&bin);
    cmd.arg("-hide_banner").arg("-encoders");
    #[cfg(target_os = "windows")]
    cmd.creation_flags(CREATE_NO_WINDOW);
    let out = cmd.output()?;
    Ok(String::from_utf8_lossy(&out.stdout).to_string())
}

/// Probe a video file: width, height, fps (rational n/d), total frame count.
/// Frame count is best-effort — falls back to duration*fps when ffprobe
/// doesn't have nb_frames in the container metadata.
#[derive(Debug, Clone, serde::Serialize)]
pub struct VideoProbe {
    pub width: u32,
    pub height: u32,
    pub fps_num: u32,
    pub fps_den: u32,
    pub total_frames: u64,
    pub duration_sec: f64,
}

pub fn probe(path: &std::path::Path) -> Result<VideoProbe> {
    let bin = ffprobe_path()?;
    let mut cmd = Command::new(&bin);
    cmd.args([
        "-v", "error",
        "-select_streams", "v:0",
        "-show_entries", "stream=width,height,r_frame_rate,nb_frames,duration",
        "-of", "default=noprint_wrappers=1",
    ]).arg(path);
    #[cfg(target_os = "windows")]
    cmd.creation_flags(CREATE_NO_WINDOW);
    let out = cmd.output()?;
    let raw = String::from_utf8_lossy(&out.stdout);

    let mut width = 0u32;
    let mut height = 0u32;
    let mut fps_num = 0u32;
    let mut fps_den = 1u32;
    let mut nb_frames = 0u64;
    let mut duration = 0f64;

    for line in raw.lines() {
        let (k, v) = match line.split_once('=') { Some(x) => x, None => continue };
        match k {
            "width"  => width = v.trim().parse().unwrap_or(0),
            "height" => height = v.trim().parse().unwrap_or(0),
            "r_frame_rate" => {
                if let Some((n, d)) = v.trim().split_once('/') {
                    fps_num = n.parse().unwrap_or(0);
                    fps_den = d.parse().unwrap_or(1).max(1);
                }
            }
            "nb_frames" => nb_frames = v.trim().parse().unwrap_or(0),
            "duration"  => duration = v.trim().parse().unwrap_or(0.0),
            _ => {}
        }
    }

    if width == 0 || height == 0 || fps_num == 0 {
        bail!("ffprobe returned incomplete metadata for {}", path.display());
    }
    if nb_frames == 0 && duration > 0.0 {
        nb_frames = (duration * fps_num as f64 / fps_den as f64).round() as u64;
    }
    Ok(VideoProbe { width, height, fps_num, fps_den, total_frames: nb_frames, duration_sec: duration })
}
