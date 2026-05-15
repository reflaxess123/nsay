// Super-resolution job runner. Mirrors tools::rembg, just spawns
// nsay-upscale-<backend>.exe and routes a different event namespace.
//
// Output naming follows the same "single → next to source / batch →
// nsay_upscale/ subfolder" rule used by rembg, with the suffix `_x4`.

use std::collections::HashMap;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;
#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

use tauri::Emitter;

use crate::config;
use crate::tools;

#[derive(serde::Serialize, Clone)]
pub struct ItemResult {
    pub idx: usize,
    pub input: String,
    pub output: Option<String>,
    pub error: Option<String>,
}

#[tauri::command]
pub async fn upscale_run(
    inputs: Vec<String>,
    scale: Option<f32>,
    model: Option<String>,
    state: tauri::State<'_, crate::state_cmd::AppState>,
    models_state: tauri::State<'_, crate::models_cmd::ModelState>,
    app: tauri::AppHandle,
) -> Result<Vec<ItemResult>, String> {
    if inputs.is_empty() {
        return Ok(Vec::new());
    }
    tracing::info!("upscale_run cmd: scale={:?} model={:?} n={}", scale, model, inputs.len());
    // Default to x4 for callers (and old configs) that don't specify.
    let scale = scale.unwrap_or(4.0).clamp(1.0, 4.0);
    let backend_choice = state.backend_choice.lock().unwrap().clone();
    let models_state_cloned = (*models_state).clone();
    tauri::async_runtime::spawn_blocking(move || {
        upscale_run_blocking(inputs, scale, model, backend_choice, models_state_cloned, app)
    })
    .await
    .map_err(|e| format!("upscale join failed: {e}"))?
}

fn upscale_run_blocking(
    inputs: Vec<String>,
    scale: f32,
    model_override: Option<String>,
    backend_choice: String,
    models_state: crate::models_cmd::ModelState,
    app: tauri::AppHandle,
) -> Result<Vec<ItemResult>, String> {
    let (backend, sidecar) =
        tools::resolve_sidecar("upscale", &backend_choice).map_err(|e| e.to_string())?;

    // Per-call override (UI dropdown) > nsay.toml > catalogue default.
    let cfg = config::Config::load().map_err(|e| e.to_string())?;
    let model_id = model_override
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| {
            if cfg.upscale.model.is_empty() {
                "real-esrgan-x4".to_string()
            } else {
                cfg.upscale.model.clone()
            }
        });
    let model_path = crate::models_cmd::ensure_model(&model_id, &models_state, &app)
        .map_err(|e| format!("model {} could not be obtained: {}", model_id, e))?;
    // model_scale = the model's native ratio (2 or 4). Sidecar uses it to
    // pick a pre-resize ratio so source × scale == source × pre × native.
    let model_scale = crate::models::find(&model_id)
        .map(|m| m.output_scale)
        .filter(|s| *s > 0)
        .unwrap_or(4);

    let total = inputs.len();
    let _ = app.emit(
        "upscale-batch-start",
        serde_json::json!({ "total": total }),
    );
    let outputs = derive_output_paths(&inputs, scale);

    let mut results: Vec<ItemResult> = Vec::with_capacity(total);
    let mut ok = 0usize;
    let mut failed = 0usize;

    for (idx, (input_str, output_pb)) in inputs.iter().zip(outputs.iter()).enumerate() {
        let input_pb = PathBuf::from(input_str);

        if !input_pb.exists() {
            let err = format!("input file not found: {}", input_str);
            let _ = app.emit(
                "upscale-item-error",
                serde_json::json!({ "idx": idx, "error": err }),
            );
            results.push(ItemResult {
                idx,
                input: input_str.clone(),
                output: None,
                error: Some(err),
            });
            failed += 1;
            continue;
        }

        if let Some(parent) = output_pb.parent() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                let err = format!("failed to create output dir {}: {}", parent.display(), e);
                let _ = app.emit(
                    "upscale-item-error",
                    serde_json::json!({ "idx": idx, "error": err }),
                );
                results.push(ItemResult {
                    idx,
                    input: input_str.clone(),
                    output: None,
                    error: Some(err),
                });
                failed += 1;
                continue;
            }
        }

        let _ = app.emit(
            "upscale-item-start",
            serde_json::json!({
                "idx": idx,
                "input": input_str,
                "output": output_pb.to_string_lossy(),
            }),
        );

        tracing::info!(
            "upscale [{}/{}] via {} | scale=x{} model_scale={} | in={} | out={}",
            idx + 1,
            total,
            backend,
            scale,
            model_scale,
            input_str,
            output_pb.display()
        );

        match run_one(&sidecar, &model_path, &input_pb, output_pb, scale, model_scale, idx, &app) {
            Ok(()) => {
                let _ = app.emit(
                    "upscale-item-done",
                    serde_json::json!({
                        "idx": idx,
                        "output": output_pb.to_string_lossy(),
                    }),
                );
                results.push(ItemResult {
                    idx,
                    input: input_str.clone(),
                    output: Some(output_pb.to_string_lossy().into_owned()),
                    error: None,
                });
                ok += 1;
            }
            Err(e) => {
                let _ = app.emit(
                    "upscale-item-error",
                    serde_json::json!({ "idx": idx, "error": e.clone() }),
                );
                results.push(ItemResult {
                    idx,
                    input: input_str.clone(),
                    output: None,
                    error: Some(e),
                });
                failed += 1;
            }
        }
    }

    let _ = app.emit(
        "upscale-batch-done",
        serde_json::json!({ "total": total, "ok": ok, "failed": failed }),
    );

    Ok(results)
}

fn run_one(
    sidecar: &Path,
    model_path: &Path,
    input_pb: &Path,
    output_pb: &Path,
    scale: f32,
    model_scale: u32,
    idx: usize,
    app: &tauri::AppHandle,
) -> Result<(), String> {
    let mut cmd = Command::new(sidecar);
    cmd.arg("--model")
        .arg(model_path)
        .arg("--input")
        .arg(input_pb)
        .arg("--output")
        .arg(output_pb)
        .arg("--scale")
        // {} on f32 prints "2" for 2.0, "1.5" for 1.5 — fine for argv.
        .arg(format!("{}", scale))
        .arg("--model-scale")
        .arg(model_scale.to_string())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    #[cfg(target_os = "windows")]
    cmd.creation_flags(CREATE_NO_WINDOW);

    let mut child = cmd
        .spawn()
        .map_err(|e| format!("failed to spawn sidecar {:?}: {}", sidecar, e))?;

    if let Some(stderr) = child.stderr.take() {
        let app_handle = app.clone();
        std::thread::spawn(move || {
            let reader = BufReader::new(stderr);
            for line in reader.lines().map_while(Result::ok) {
                if let Some((_stage, pct)) = parse_progress(&line) {
                    let _ = app_handle.emit(
                        "upscale-item-progress",
                        serde_json::json!({ "idx": idx, "pct": pct }),
                    );
                } else {
                    // INFO so non-progress lines (warnings, errors, ort
                    // verbose output) survive the default log level —
                    // they're how we diagnose silent stalls.
                    tracing::info!("upscale[{}] sidecar: {}", idx, line);
                }
            }
        });
    }
    if let Some(stdout) = child.stdout.take() {
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                tracing::debug!("upscale stdout: {}", line);
            }
        });
    }

    let status = child.wait().map_err(|e| e.to_string())?;
    if !status.success() {
        return Err(format!("upscale sidecar exited with {:?}", status.code()));
    }
    Ok(())
}

fn parse_progress(line: &str) -> Option<(String, u8)> {
    let mut stage = None;
    let mut pct = None;
    for token in line.split_whitespace() {
        if let Some(v) = token.strip_prefix("stage=") {
            stage = Some(v.to_string());
        } else if let Some(v) = token.strip_prefix("pct=") {
            pct = v.parse::<u8>().ok();
        }
    }
    match (stage, pct) {
        (Some(s), Some(p)) => Some((s, p)),
        _ => None,
    }
}

fn derive_output_paths(inputs: &[String], scale: f32) -> Vec<PathBuf> {
    // "_x4.png" / "_x1_5.png" — underscore avoids a stray dot in the stem
    // that some tools (and casual eyes) misread as the file extension.
    let scale_tag = if (scale - scale.round()).abs() < f32::EPSILON {
        format!("{}", scale as u32)
    } else {
        format!("{}", scale).replace('.', "_")
    };
    let pbs: Vec<PathBuf> = inputs.iter().map(PathBuf::from).collect();

    let mut counts: HashMap<PathBuf, usize> = HashMap::new();
    for p in &pbs {
        let parent = p
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| PathBuf::from("."));
        *counts.entry(parent).or_insert(0) += 1;
    }

    pbs.iter()
        .map(|p| {
            let parent = p
                .parent()
                .map(|p| p.to_path_buf())
                .unwrap_or_else(|| PathBuf::from("."));
            let stem = p
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_else(|| "output".into());
            let group_size = counts.get(&parent).copied().unwrap_or(1);
            if group_size <= 1 {
                parent.join(format!("{}_x{}.png", stem, scale_tag))
            } else {
                parent.join("nsay_upscale").join(format!("{}.png", stem))
            }
        })
        .collect()
}
