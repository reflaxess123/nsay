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
        "vid-start",
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
    let app_for_progress = app.clone();
    let total = probe.total_frames;
    spawn_progress("sidecar", side_proc.stderr.take().unwrap(), app_for_progress, total);

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

    // ---- wait all three; aggregate exit codes ----
    let read_status = read_proc.wait().map_err(|e| e.to_string())?;
    let side_status = side_proc.wait().map_err(|e| e.to_string())?;
    let write_status = write_proc.wait().map_err(|e| e.to_string())?;

    if !read_status.success() {
        return Err(format!("ffmpeg-read exited {:?}", read_status.code()));
    }
    if !side_status.success() {
        return Err(format!("sidecar exited {:?}", side_status.code()));
    }
    if !write_status.success() {
        return Err(format!("ffmpeg-write exited {:?}", write_status.code()));
    }

    let result = VideoResult {
        output: output_pb.to_string_lossy().into_owned(),
        frames: probe.total_frames,
        backend: backend.clone(),
        encoder: encoder.family.to_string(),
    };
    let _ = app.emit("vid-done", &result);
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
        "vid-start",
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
    spawn_progress("interp", side_proc.stderr.take().unwrap(), app.clone(), out_frames);

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

    let read_status = read_proc.wait().map_err(|e| e.to_string())?;
    let side_status = side_proc.wait().map_err(|e| e.to_string())?;
    let write_status = write_proc.wait().map_err(|e| e.to_string())?;

    if !read_status.success() {
        return Err(format!("ffmpeg-read exited {:?}", read_status.code()));
    }
    if !side_status.success() {
        return Err(format!("interp sidecar exited {:?}", side_status.code()));
    }
    if !write_status.success() {
        return Err(format!("ffmpeg-write exited {:?}", write_status.code()));
    }

    let result = VideoResult {
        output: output_pb.to_string_lossy().into_owned(),
        frames: out_frames,
        backend: backend.clone(),
        encoder: encoder.family.to_string(),
    };
    let _ = app.emit("vid-done", &result);
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

/// Parse sidecar stderr for `frame N` lines and emit `vid-progress`
/// events. Logs everything else (warnings, EP messages) at INFO.
fn spawn_progress(
    label: &'static str,
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
                        "vid-progress",
                        serde_json::json!({ "frame": n, "total": total, "pct": pct }),
                    );
                    continue;
                }
            }
            tracing::info!("{}: {}", label, line);
        }
    });
}
