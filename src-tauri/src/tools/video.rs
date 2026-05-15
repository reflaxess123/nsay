// Video pipeline runner. Spawns three processes wired through OS pipes:
//
//   ffmpeg(read raw RGB) ─stdout→stdin─ sidecar(stream) ─stdout→stdin─ ffmpeg(encode + audio mux)
//
// On Windows std::process::Stdio::from(ChildStdout) yields a HANDLE that can
// be assigned as another child's stdin → no per-byte copy in Rust, the OS
// just rejoins the pipe ends.
//
// The sidecar's stderr is parsed for `frame N` lines (emitted after each
// frame) and turned into vid-progress events. ffmpeg's stderr is logged at
// INFO so failures surface in nsay.log without flooding the UI.

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;
#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

use tauri::Emitter;

use crate::ffmpeg;
use crate::tools;

#[derive(serde::Serialize, Clone)]
pub struct VideoResult {
    pub output: String,
    pub frames: u64,
    pub backend: String,
    pub encoder: String,
}

/// Tauri command — async because spawn_blocking holds the IPC thread for
/// many minutes on a real video.
#[tauri::command]
pub async fn video_upscale_run(
    input: String,
    output: String,
    scale: Option<f32>,
    model: Option<String>,
    state: tauri::State<'_, crate::state_cmd::AppState>,
    models_state: tauri::State<'_, crate::models_cmd::ModelState>,
    app: tauri::AppHandle,
) -> Result<VideoResult, String> {
    let scale = scale.unwrap_or(2.0).clamp(1.0, 4.0);
    let backend_choice = state.backend_choice.lock().unwrap().clone();
    let models_state_cloned = (*models_state).clone();
    tauri::async_runtime::spawn_blocking(move || {
        run_blocking(input, output, scale, model, backend_choice, models_state_cloned, app)
    })
    .await
    .map_err(|e| format!("video join failed: {e}"))?
}

/// True video super-resolution. Two backends share this command:
///   - cpu/cuda  → libtorch + RealBasicVSR (3-process streaming pipeline)
///   - docker    → FlashVSR-Pro container (single-process file→file)
///
/// `window` only matters for libtorch (RealBasicVSR clip size).
/// `mode` / `tile_vae` / `tile_dit` / `keep_audio` only matter for docker
/// (FlashVSR-Pro tuning knobs). Each backend ignores the other's params.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn video_vidsr_run(
    input: String,
    output: String,
    scale: Option<u32>,
    window: Option<u32>,
    model: Option<String>,
    mode: Option<String>,
    tile_vae: Option<bool>,
    tile_dit: Option<bool>,
    keep_audio: Option<bool>,
    state: tauri::State<'_, crate::state_cmd::AppState>,
    models_state: tauri::State<'_, crate::models_cmd::ModelState>,
    app: tauri::AppHandle,
) -> Result<VideoResult, String> {
    let scale = scale.unwrap_or(4).clamp(2, 4);
    let window = window.unwrap_or(15).clamp(3, 30);
    let mode = mode.unwrap_or_else(|| "tiny".to_string());
    let tile_vae = tile_vae.unwrap_or(true);
    let tile_dit = tile_dit.unwrap_or(true);
    let keep_audio = keep_audio.unwrap_or(true);
    let backend_choice = state.backend_choice.lock().unwrap().clone();
    let models_state_cloned = (*models_state).clone();
    tauri::async_runtime::spawn_blocking(move || {
        run_vidsr_blocking(
            input, output, scale, window, model,
            mode, tile_vae, tile_dit, keep_audio,
            backend_choice, models_state_cloned, app,
        )
    })
    .await
    .map_err(|e| format!("video-vidsr join failed: {e}"))?
}

/// RIFE frame interpolation. Two output modes:
///   - "boost" (default): same duration, fps × factor, audio kept.
///                        e.g. 30fps@10s → 60fps@10s.
///   - "slow":           duration × factor, fps unchanged, audio dropped.
///                        e.g. 30fps@10s → 30fps@20s. Slow-mo classic.
#[tauri::command]
pub async fn video_interp_run(
    input: String,
    output: String,
    factor: Option<u32>,
    mode: Option<String>,
    model: Option<String>,
    state: tauri::State<'_, crate::state_cmd::AppState>,
    models_state: tauri::State<'_, crate::models_cmd::ModelState>,
    app: tauri::AppHandle,
) -> Result<VideoResult, String> {
    let factor = factor.unwrap_or(2).clamp(2, 16);
    let mode = mode.unwrap_or_else(|| "boost".to_string());
    let backend_choice = state.backend_choice.lock().unwrap().clone();
    let models_state_cloned = (*models_state).clone();
    tauri::async_runtime::spawn_blocking(move || {
        run_interp_blocking(input, output, factor, mode, model, backend_choice, models_state_cloned, app)
    })
    .await
    .map_err(|e| format!("video-interp join failed: {e}"))?
}

fn run_blocking(
    input: String,
    output: String,
    scale: f32,
    model_override: Option<String>,
    backend_choice: String,
    models_state: crate::models_cmd::ModelState,
    app: tauri::AppHandle,
) -> Result<VideoResult, String> {
    let input_pb = PathBuf::from(&input);
    let output_pb = PathBuf::from(&output);
    if !input_pb.exists() {
        return Err(format!("input not found: {}", input_pb.display()));
    }
    if let Some(parent) = output_pb.parent() {
        std::fs::create_dir_all(parent).ok();
    }

    // 1. resolve sidecar (stream-mode capable)
    let (backend, sidecar_path) = tools::resolve_sidecar("upscale", &backend_choice)
        .map_err(|e| e.to_string())?;

    // 2. resolve / download model
    let cfg = crate::config::Config::load().map_err(|e| e.to_string())?;
    let model_id = model_override.filter(|s| !s.is_empty()).unwrap_or_else(|| {
        if cfg.upscale.model.is_empty() { "real-esrgan-x4".to_string() } else { cfg.upscale.model.clone() }
    });
    let model_path = crate::models_cmd::ensure_model(&model_id, &models_state, &app)
        .map_err(|e| format!("model {} could not be obtained: {}", model_id, e))?;
    let model_scale = crate::models::find(&model_id)
        .map(|m| m.output_scale).filter(|s| *s > 0).unwrap_or(4);

    // 3. probe input video
    let probe = ffmpeg::probe(&input_pb).map_err(|e| format!("ffprobe: {e}"))?;
    tracing::info!(
        "video_upscale: in={} {}x{} fps={}/{} frames~={} | backend={} scale=x{} model={} model_scale={}",
        input, probe.width, probe.height, probe.fps_num, probe.fps_den,
        probe.total_frames, backend, scale, model_id, model_scale,
    );

    // 4. encoder pick
    let encoder = ffmpeg::detect_encoder();
    tracing::info!("video_upscale: encoder family={} h264={}", encoder.family, encoder.h264);

    // 5. compute output dims (must match sidecar's internal arithmetic)
    let new_w = ((probe.width as f32 * scale / model_scale as f32).round() as u32).max(1);
    let new_h = ((probe.height as f32 * scale / model_scale as f32).round() as u32).max(1);
    let out_w = new_w * model_scale;
    let out_h = new_h * model_scale;

    let _ = app.emit(
        "vid-upscale-start",
        serde_json::json!({
            "total_frames": probe.total_frames,
            "src_w": probe.width, "src_h": probe.height,
            "out_w": out_w, "out_h": out_h,
            "fps_num": probe.fps_num, "fps_den": probe.fps_den,
            "backend": backend, "encoder": encoder.family,
        }),
    );

    let ffmpeg_bin = ffmpeg::ffmpeg_path().map_err(|e| e.to_string())?;

    // ---- stage 1: ffmpeg read (decode → raw RGB) ----
    let mut read = build_cmd(&ffmpeg_bin);
    read.args([
        "-hide_banner", "-loglevel", "error",
        "-i",
    ]).arg(&input_pb).args([
        "-f", "rawvideo",
        "-pix_fmt", "rgb24",
        "-",
    ]).stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut read_proc = read.spawn().map_err(|e| format!("spawn ffmpeg-read: {e}"))?;
    spawn_logger("ffmpeg-read", read_proc.stderr.take().unwrap());
    let read_stdout = read_proc.stdout.take().unwrap();

    // ---- stage 2: sidecar in stream mode ----
    let mut side = build_cmd(&sidecar_path);
    side.arg("--stream")
        .arg("--width").arg(probe.width.to_string())
        .arg("--height").arg(probe.height.to_string())
        .arg("--scale").arg(format!("{}", scale))
        .arg("--model-scale").arg(model_scale.to_string())
        .arg("--model").arg(&model_path)
        .stdin(Stdio::from(read_stdout))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut side_proc = side.spawn().map_err(|e| format!("spawn sidecar: {e}"))?;
    let side_stdout = side_proc.stdout.take().unwrap();
    let total = probe.total_frames;
    spawn_progress("sidecar", "vid-upscale-progress", side_proc.stderr.take().unwrap(), app.clone(), total);

    // ---- stage 3: ffmpeg write (encode + audio mux from source) ----
    let fps_str = format!("{}/{}", probe.fps_num, probe.fps_den);
    let dims_str = format!("{}x{}", out_w, out_h);
    let mut write = build_cmd(&ffmpeg_bin);
    write.args([
        "-hide_banner", "-loglevel", "error", "-y",
        // upscaled raw RGB stream (from sidecar) — input #0
        "-framerate", &fps_str,
        "-f", "rawvideo",
        "-pix_fmt", "rgb24",
        "-s", &dims_str,
        "-i", "-",
        // audio source — input #1
        "-i",
    ]).arg(&input_pb).args([
        // map upscaled video + optional audio from source. `?` makes the
        // audio map optional (some inputs have no audio track).
        "-map", "0:v",
        "-map", "1:a?",
        "-c:v", encoder.h264,
        "-pix_fmt", "yuv420p",   // browser-compatible output
        "-c:a", "copy",          // don't re-encode audio
        "-shortest",
    ]).arg(&output_pb)
        .stdin(Stdio::from(side_stdout))
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    let mut write_proc = write.spawn().map_err(|e| format!("spawn ffmpeg-write: {e}"))?;
    spawn_logger("ffmpeg-write", write_proc.stderr.take().unwrap());

    // Parallel wait — first non-zero exit kills the other two so we don't
    // hang on a doomed pipe (sidecar panic → read blocked on EPIPE).
    wait_pipeline(read_proc, side_proc, write_proc)?;

    let result = VideoResult {
        output: output_pb.to_string_lossy().into_owned(),
        frames: probe.total_frames,
        backend: backend.clone(),
        encoder: encoder.family.to_string(),
    };
    let _ = app.emit("vid-upscale-done", &result);
    Ok(result)
}

fn run_interp_blocking(
    input: String,
    output: String,
    factor: u32,
    mode: String,
    model_override: Option<String>,
    backend_choice: String,
    models_state: crate::models_cmd::ModelState,
    app: tauri::AppHandle,
) -> Result<VideoResult, String> {
    let slow = mode.eq_ignore_ascii_case("slow");
    let input_pb = PathBuf::from(&input);
    let output_pb = PathBuf::from(&output);
    if !input_pb.exists() {
        return Err(format!("input not found: {}", input_pb.display()));
    }
    if let Some(parent) = output_pb.parent() {
        std::fs::create_dir_all(parent).ok();
    }

    let (backend, sidecar_path) = tools::resolve_sidecar("interp", &backend_choice)
        .map_err(|e| e.to_string())?;

    let cfg = crate::config::Config::load().map_err(|e| e.to_string())?;
    let model_id = model_override.filter(|s| !s.is_empty()).unwrap_or_else(|| {
        if cfg.interp.model.is_empty() { "rife-4.9".to_string() } else { cfg.interp.model.clone() }
    });
    let model_path = crate::models_cmd::ensure_model(&model_id, &models_state, &app)
        .map_err(|e| format!("model {} could not be obtained: {}", model_id, e))?;

    let probe = ffmpeg::probe(&input_pb).map_err(|e| format!("ffprobe: {e}"))?;
    let encoder = ffmpeg::detect_encoder();

    // Boost: fps × factor (same wall-clock duration).
    // Slow:  fps unchanged (duration × factor — frames are spread over
    //        more seconds at the original frame rate).
    // Numerator-only multiply keeps the rational clean (30000/1001 × 2
    // → 60000/1001 = 59.94 fps).
    let (out_fps_num, out_fps_den) = if slow {
        (probe.fps_num, probe.fps_den)
    } else {
        (probe.fps_num.saturating_mul(factor), probe.fps_den)
    };
    // Output frame count: 1 + (N-1)*K (sidecar emits first frame once,
    // then K frames per pair).
    let out_frames = 1 + probe.total_frames.saturating_sub(1) * factor as u64;

    tracing::info!(
        "video_interp: in={} {}x{} fps={}/{} N~={} | backend={} factor=x{} mode={} out_fps={}/{} N_out~={}",
        input, probe.width, probe.height, probe.fps_num, probe.fps_den,
        probe.total_frames, backend, factor, if slow { "slow" } else { "boost" },
        out_fps_num, out_fps_den, out_frames,
    );

    let _ = app.emit(
        "vid-interp-start",
        serde_json::json!({
            "total_frames": out_frames,
            "src_w": probe.width, "src_h": probe.height,
            "out_w": probe.width, "out_h": probe.height,
            "fps_num": out_fps_num, "fps_den": out_fps_den,
            "backend": backend, "encoder": encoder.family,
        }),
    );

    let ffmpeg_bin = ffmpeg::ffmpeg_path().map_err(|e| e.to_string())?;

    // ---- stage 1: ffmpeg read ----
    let mut read = build_cmd(&ffmpeg_bin);
    read.args([
        "-hide_banner", "-loglevel", "error",
        "-i",
    ]).arg(&input_pb).args([
        "-f", "rawvideo",
        "-pix_fmt", "rgb24",
        "-",
    ]).stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut read_proc = read.spawn().map_err(|e| format!("spawn ffmpeg-read: {e}"))?;
    spawn_logger("ffmpeg-read", read_proc.stderr.take().unwrap());
    let read_stdout = read_proc.stdout.take().unwrap();

    // ---- stage 2: interp sidecar ----
    let mut side = build_cmd(&sidecar_path);
    side.arg("--stream")
        .arg("--width").arg(probe.width.to_string())
        .arg("--height").arg(probe.height.to_string())
        .arg("--factor").arg(factor.to_string())
        .arg("--model").arg(&model_path)
        .stdin(Stdio::from(read_stdout))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut side_proc = side.spawn().map_err(|e| format!("spawn sidecar: {e}"))?;
    let side_stdout = side_proc.stdout.take().unwrap();
    spawn_progress("interp", "vid-interp-progress", side_proc.stderr.take().unwrap(), app.clone(), out_frames);

    // ---- stage 3: ffmpeg write ----
    // Boost: keep audio (`-map 1:a? -c:a copy`) — duration matches source.
    // Slow:  audio is dropped (`-an`) — output runs longer than source
    //        audio, so copying would either truncate (`-shortest`) or
    //        leave silent tail. Time-stretching audio (atempo) is a
    //        deliberate creative choice and not what slow-mo usually wants.
    let fps_str = format!("{}/{}", out_fps_num, out_fps_den);
    let dims_str = format!("{}x{}", probe.width, probe.height);
    let mut write = build_cmd(&ffmpeg_bin);
    write.args([
        "-hide_banner", "-loglevel", "error", "-y",
        "-framerate", &fps_str,
        "-f", "rawvideo",
        "-pix_fmt", "rgb24",
        "-s", &dims_str,
        "-i", "-",
    ]);
    if !slow {
        // Audio source — input #1, only when keeping audio.
        write.arg("-i").arg(&input_pb);
        write.args([
            "-map", "0:v",
            "-map", "1:a?",
            "-c:v", encoder.h264,
            "-pix_fmt", "yuv420p",
            "-c:a", "copy",
            "-shortest",
        ]);
    } else {
        write.args([
            "-map", "0:v",
            "-an",
            "-c:v", encoder.h264,
            "-pix_fmt", "yuv420p",
        ]);
    }
    write.arg(&output_pb)
        .stdin(Stdio::from(side_stdout))
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    let mut write_proc = write.spawn().map_err(|e| format!("spawn ffmpeg-write: {e}"))?;
    spawn_logger("ffmpeg-write", write_proc.stderr.take().unwrap());

    wait_pipeline(read_proc, side_proc, write_proc)?;

    let result = VideoResult {
        output: output_pb.to_string_lossy().into_owned(),
        frames: out_frames,
        backend: backend.clone(),
        encoder: encoder.family.to_string(),
    };
    let _ = app.emit("vid-interp-done", &result);
    Ok(result)
}

#[allow(clippy::too_many_arguments)]
fn run_vidsr_blocking(
    input: String,
    output: String,
    scale: u32,
    window: u32,
    model_override: Option<String>,
    mode: String,
    tile_vae: bool,
    tile_dit: bool,
    keep_audio: bool,
    backend_choice: String,
    models_state: crate::models_cmd::ModelState,
    app: tauri::AppHandle,
) -> Result<VideoResult, String> {
    let input_pb = PathBuf::from(&input);
    let output_pb = PathBuf::from(&output);
    if !input_pb.exists() {
        return Err(format!("input not found: {}", input_pb.display()));
    }
    if let Some(parent) = output_pb.parent() {
        std::fs::create_dir_all(parent).ok();
    }

    let (backend, sidecar_path) = tools::resolve_sidecar("vidsr", &backend_choice)
        .map_err(|e| e.to_string())?;

    let probe = ffmpeg::probe(&input_pb).map_err(|e| format!("ffprobe: {e}"))?;
    let encoder = ffmpeg::detect_encoder();
    let out_w = probe.width * scale;
    let out_h = probe.height * scale;

    let _ = app.emit(
        "vid-vidsr-start",
        serde_json::json!({
            "total_frames": probe.total_frames,
            "src_w": probe.width, "src_h": probe.height,
            "out_w": out_w, "out_h": out_h,
            "fps_num": probe.fps_num, "fps_den": probe.fps_den,
            "backend": backend, "encoder": encoder.family,
        }),
    );

    // Docker backend = FlashVSR-Pro container, file→file. The shim handles
    // its own ffmpeg/encode inside the image, so we don't build a 3-process
    // pipeline here — just spawn the sidecar and forward its tqdm progress.
    if backend == "docker" {
        tracing::info!(
            "video_vidsr (docker): in={} {}x{} N~={} | scale=x{} mode={} tile_vae={} tile_dit={} keep_audio={}",
            input, probe.width, probe.height, probe.total_frames,
            scale, mode, tile_vae, tile_dit, keep_audio,
        );
        return run_vidsr_docker_blocking(
            &input_pb, &output_pb, scale,
            &mode, tile_vae, tile_dit, keep_audio,
            &sidecar_path, &backend, &encoder.family,
            probe.total_frames, app,
        );
    }

    // libtorch path needs a model file on disk; docker doesn't.
    let cfg = crate::config::Config::load().map_err(|e| e.to_string())?;
    let model_id = model_override.filter(|s| !s.is_empty()).unwrap_or_else(|| {
        if cfg.vidsr.model.is_empty() { "realbasicvsr-x4".to_string() } else { cfg.vidsr.model.clone() }
    });
    let model_path = crate::models_cmd::ensure_model(&model_id, &models_state, &app)
        .map_err(|e| format!("model {} could not be obtained: {}", model_id, e))?;

    tracing::info!(
        "video_vidsr (libtorch): in={} {}x{} fps={}/{} N~={} | backend={} scale=x{} window={} model={}",
        input, probe.width, probe.height, probe.fps_num, probe.fps_den,
        probe.total_frames, backend, scale, window, model_id,
    );

    let ffmpeg_bin = ffmpeg::ffmpeg_path().map_err(|e| e.to_string())?;

    // Stage 1: ffmpeg decode → raw RGB.
    let mut read = build_cmd(&ffmpeg_bin);
    read.args([
        "-hide_banner", "-loglevel", "error",
        "-i",
    ]).arg(&input_pb).args([
        "-f", "rawvideo",
        "-pix_fmt", "rgb24",
        "-",
    ]).stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut read_proc = read.spawn().map_err(|e| format!("spawn ffmpeg-read: {e}"))?;
    spawn_logger("ffmpeg-read", read_proc.stderr.take().unwrap());
    let read_stdout = read_proc.stdout.take().unwrap();

    // Stage 2: vidsr sidecar (chunk-based; consumes N frames at a time).
    let mut side = build_cmd(&sidecar_path);
    side.arg("--stream")
        .arg("--width").arg(probe.width.to_string())
        .arg("--height").arg(probe.height.to_string())
        .arg("--scale").arg(scale.to_string())
        .arg("--window").arg(window.to_string())
        .arg("--model").arg(&model_path)
        .stdin(Stdio::from(read_stdout))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut side_proc = side.spawn().map_err(|e| format!("spawn sidecar: {e}"))?;
    let side_stdout = side_proc.stdout.take().unwrap();
    spawn_progress("vidsr", "vid-vidsr-progress", side_proc.stderr.take().unwrap(), app.clone(), probe.total_frames);

    // Stage 3: ffmpeg encode + audio mux from source.
    let fps_str = format!("{}/{}", probe.fps_num, probe.fps_den);
    let dims_str = format!("{}x{}", out_w, out_h);
    let mut write = build_cmd(&ffmpeg_bin);
    write.args([
        "-hide_banner", "-loglevel", "error", "-y",
        "-framerate", &fps_str,
        "-f", "rawvideo",
        "-pix_fmt", "rgb24",
        "-s", &dims_str,
        "-i", "-",
        "-i",
    ]).arg(&input_pb).args([
        "-map", "0:v",
        "-map", "1:a?",
        "-c:v", encoder.h264,
        "-pix_fmt", "yuv420p",
        "-c:a", "copy",
        "-shortest",
    ]).arg(&output_pb)
        .stdin(Stdio::from(side_stdout))
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    let mut write_proc = write.spawn().map_err(|e| format!("spawn ffmpeg-write: {e}"))?;
    spawn_logger("ffmpeg-write", write_proc.stderr.take().unwrap());

    wait_pipeline(read_proc, side_proc, write_proc)?;

    let result = VideoResult {
        output: output_pb.to_string_lossy().into_owned(),
        frames: probe.total_frames,
        backend: backend.clone(),
        encoder: encoder.family.to_string(),
    };
    let _ = app.emit("vid-vidsr-done", &result);
    Ok(result)
}

/// Docker backend for vidsr — single subprocess. The container does its own
/// ffmpeg decode/encode + GPU inference, so there's no pipe to wire up.
/// We just hand it --input/--output and let it work; progress comes back as
/// `frame N` lines from the shim (it parses tqdm internally).
#[allow(clippy::too_many_arguments)]
fn run_vidsr_docker_blocking(
    input_pb: &Path,
    output_pb: &Path,
    scale: u32,
    mode: &str,
    tile_vae: bool,
    tile_dit: bool,
    keep_audio: bool,
    sidecar_path: &Path,
    backend: &str,
    encoder_family: &str,
    total_frames: u64,
    app: tauri::AppHandle,
) -> Result<VideoResult, String> {
    let mut cmd = build_cmd(sidecar_path);
    cmd.arg("--input").arg(input_pb)
        .arg("--output").arg(output_pb)
        .arg("--scale").arg(scale.to_string())
        .arg("--mode").arg(mode);
    // Use --no-* opt-outs because the shim defaults are all true. This keeps
    // the command short in the common case (`--mode tiny --scale 4`).
    if !tile_vae   { cmd.arg("--no-tile-vae"); }
    if !tile_dit   { cmd.arg("--no-tile-dit"); }
    if !keep_audio { cmd.arg("--no-keep-audio"); }

    // Discard stdout — the shim only writes to stderr (progress + logs);
    // anything on stdout would just be noise we don't want to forward.
    cmd.stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());

    let mut child = cmd.spawn().map_err(|e| format!("spawn vidsr-docker: {e}"))?;
    spawn_progress(
        "vidsr-docker",
        "vid-vidsr-progress",
        child.stderr.take().unwrap(),
        app.clone(),
        total_frames,
    );

    let status = child.wait().map_err(|e| format!("vidsr-docker wait: {e}"))?;
    if !status.success() {
        return Err(format!("vidsr-docker exited {:?}", status.code()));
    }

    let result = VideoResult {
        output: output_pb.to_string_lossy().into_owned(),
        frames: total_frames,
        backend: backend.to_string(),
        encoder: encoder_family.to_string(),
    };
    let _ = app.emit("vid-vidsr-done", &result);
    Ok(result)
}

fn build_cmd(bin: &Path) -> Command {
    let mut cmd = Command::new(bin);
    #[cfg(target_os = "windows")]
    cmd.creation_flags(CREATE_NO_WINDOW);
    cmd
}

fn spawn_logger(label: &'static str, stderr: std::process::ChildStderr) {
    std::thread::spawn(move || {
        let reader = BufReader::new(stderr);
        for line in reader.lines().map_while(Result::ok) {
            tracing::info!("{}: {}", label, line);
        }
    });
}

/// Parse sidecar stderr for `frame N` lines and emit progress events on
/// `event_name`. Logs everything else (warnings, EP messages) at INFO.
/// `event_name` is namespaced per tool — `vid-upscale-progress` /
/// `vid-interp-progress` — so two stores listening at once don't update
/// each other's progress bars.
fn spawn_progress(
    label: &'static str,
    event_name: &'static str,
    stderr: std::process::ChildStderr,
    app: tauri::AppHandle,
    total: u64,
) {
    std::thread::spawn(move || {
        let reader = BufReader::new(stderr);
        for line in reader.lines().map_while(Result::ok) {
            if let Some(rest) = line.strip_prefix("frame ") {
                if let Ok(n) = rest.trim().parse::<u64>() {
                    let pct = if total > 0 { ((n * 100) / total).min(100) } else { 0 };
                    let _ = app.emit(
                        event_name,
                        serde_json::json!({ "frame": n, "total": total, "pct": pct }),
                    );
                    continue;
                }
            }
            tracing::info!("{}: {}", label, line);
        }
    });
}

/// Wait for three pipeline processes (read → sidecar → write) in parallel.
/// If any one fails (non-zero exit), the others are killed immediately so
/// the call returns instead of hanging on a blocked pipe.
///
/// Hang scenario this fixes: sidecar panics mid-stream → its stdout closes
/// → ffmpeg-write reads EOF and exits cleanly with the partial frames it
/// already had → ffmpeg-read's pipe to the dead sidecar fills up → read
/// blocks on write forever → sequential `read.wait()` first never returns.
/// Sequential wait also reports "ffmpeg-read exited 1" instead of the
/// real "sidecar exited" root cause, which is a misleading error.
fn wait_pipeline(
    read_proc: std::process::Child,
    side_proc: std::process::Child,
    write_proc: std::process::Child,
) -> Result<(), String> {
    use std::sync::mpsc;
    type WaitMsg = (&'static str, std::io::Result<std::process::ExitStatus>);
    let (tx, rx) = mpsc::channel::<WaitMsg>();

    // Snapshot pids so we can kill survivors even after the Child moves
    // into its waiter thread (Child::kill needs &mut, we can't share it).
    let pids = [
        ("read",  read_proc.id()),
        ("side",  side_proc.id()),
        ("write", write_proc.id()),
    ];

    for (label, mut child, sender) in [
        ("read",  read_proc,  tx.clone()),
        ("side",  side_proc,  tx.clone()),
        ("write", write_proc, tx),
    ] {
        std::thread::spawn(move || {
            let _ = sender.send((label, child.wait()));
        });
    }

    let mut remaining = 3;
    let mut killed = false;
    let mut first_failure: Option<String> = None;
    while remaining > 0 {
        let (label, status) = rx.recv().map_err(|e| format!("pipeline channel: {e}"))?;
        remaining -= 1;
        match status {
            Ok(s) if !s.success() && !killed => {
                killed = true;
                first_failure = Some(format!("{} exited {:?}", label, s.code()));
                for (l, pid) in pids.iter() {
                    if *l != label { kill_pid(*pid); }
                }
            }
            Ok(_) => {}
            Err(e) => {
                if first_failure.is_none() {
                    first_failure = Some(format!("{} wait failed: {e}", label));
                }
            }
        }
    }
    match first_failure {
        Some(msg) => Err(msg),
        None => Ok(()),
    }
}

#[cfg(target_os = "windows")]
fn kill_pid(pid: u32) {
    let _ = std::process::Command::new("taskkill")
        .args(["/F", "/T", "/PID", &pid.to_string()])
        .creation_flags(CREATE_NO_WINDOW)
        .status();
}

#[cfg(not(target_os = "windows"))]
fn kill_pid(pid: u32) {
    // POSIX: shell out to kill instead of pulling in libc just for SIGKILL.
    let _ = std::process::Command::new("kill")
        .args(["-9", &pid.to_string()])
        .status();
}
