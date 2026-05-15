// Background removal job runner. Spawns nsay-rembg-<backend>.exe per
// input image; emits per-item progress events the UI uses to drive a
// grid of thumbnails (one card per file).
//
// Output naming follows the user's intent:
//   - 1 file in a parent dir   → <parent>/<stem>_rembg.png  (next to source)
//   - N files in same parent   → <parent>/nsay_rembg/<stem>.png
//   - mixed parents            → each group above applied independently
//
// Events (all carry `idx` to identify which UI tile to update):
//   rembg-batch-start { total }
//   rembg-item-start    { idx, input, output }
//   rembg-item-progress { idx, pct }
//   rembg-item-done     { idx, output }
//   rembg-item-error    { idx, error }
//   rembg-batch-done    { total, ok, failed }

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

/// `async` so child.wait() in the worker doesn't pin the IPC thread —
/// per-item events would otherwise queue up and arrive in a single burst.
#[tauri::command]
pub async fn rembg_run(
    inputs: Vec<String>,
    choke: f32,
    model: Option<String>,
    state: tauri::State<'_, crate::state_cmd::AppState>,
    models_state: tauri::State<'_, crate::models_cmd::ModelState>,
    app: tauri::AppHandle,
) -> Result<Vec<ItemResult>, String> {
    if inputs.is_empty() {
        return Ok(Vec::new());
    }
    let backend_choice = state.backend_choice.lock().unwrap().clone();
    let models_state_cloned = (*models_state).clone();
    tauri::async_runtime::spawn_blocking(move || {
        rembg_run_blocking(inputs, choke, model, backend_choice, models_state_cloned, app)
    })
    .await
    .map_err(|e| format!("rembg join failed: {e}"))?
}

fn rembg_run_blocking(
    inputs: Vec<String>,
    choke: f32,
    model_override: Option<String>,
    backend_choice: String,
    models_state: crate::models_cmd::ModelState,
    app: tauri::AppHandle,
) -> Result<Vec<ItemResult>, String> {
    let (backend, sidecar) =
        tools::resolve_sidecar("rembg", &backend_choice).map_err(|e| e.to_string())?;

    // Per-call override (UI dropdown) > nsay.toml > catalogue default.
    let cfg = config::Config::load().map_err(|e| e.to_string())?;
    let model_id = model_override
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| {
            if cfg.rembg.model.is_empty() {
                "bria-rmbg-1.4".to_string()
            } else {
                cfg.rembg.model.clone()
            }
        });
    // ensure_model: noop if file is on disk; otherwise streams from HF
    // and emits model-download-{start,progress,done,error}. Done once
    // per batch up front so the user doesn't see download events
    // interleaved with per-item progress later.
    let model_path = crate::models_cmd::ensure_model(&model_id, &models_state, &app)
        .map_err(|e| format!("model {} could not be obtained: {}", model_id, e))?;

    // Resolve each input → output path, grouping by parent dir.
    let total = inputs.len();
    let _ = app.emit(
        "rembg-batch-start",
        serde_json::json!({ "total": total }),
    );
    let outputs = derive_output_paths(&inputs);

    let mut results: Vec<ItemResult> = Vec::with_capacity(total);
    let mut ok = 0usize;
    let mut failed = 0usize;

    for (idx, (input_str, output_pb)) in inputs.iter().zip(outputs.iter()).enumerate() {
        let input_pb = PathBuf::from(input_str);

        // Skip files that vanished between drag-drop and now.
        if !input_pb.exists() {
            let err = format!("input file not found: {}", input_str);
            let _ = app.emit(
                "rembg-item-error",
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

        // Make sure the output dir exists (matters for the nsay_rembg/
        // sub-folder case).
        if let Some(parent) = output_pb.parent() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                let err = format!("failed to create output dir {}: {}", parent.display(), e);
                let _ = app.emit(
                    "rembg-item-error",
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
            "rembg-item-start",
            serde_json::json!({
                "idx": idx,
                "input": input_str,
                "output": output_pb.to_string_lossy(),
            }),
        );

        tracing::info!(
            "rembg [{}/{}] via {} | in={} | out={}",
            idx + 1,
            total,
            backend,
            input_str,
            output_pb.display()
        );

        match run_one(&sidecar, &model_path, &input_pb, output_pb, choke, idx, &app) {
            Ok(()) => {
                let _ = app.emit(
                    "rembg-item-done",
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
                    "rembg-item-error",
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
        "rembg-batch-done",
        serde_json::json!({ "total": total, "ok": ok, "failed": failed }),
    );

    Ok(results)
}

fn run_one(
    sidecar: &Path,
    model_path: &Path,
    input_pb: &Path,
    output_pb: &Path,
    choke: f32,
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
        .arg("--choke")
        .arg(format!("{:.4}", choke))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    #[cfg(target_os = "windows")]
    cmd.creation_flags(CREATE_NO_WINDOW);

    let mut child = cmd
        .spawn()
        .map_err(|e| format!("failed to spawn sidecar {:?}: {}", sidecar, e))?;

    // Forward sidecar stderr → per-item progress events.
    if let Some(stderr) = child.stderr.take() {
        let app_handle = app.clone();
        std::thread::spawn(move || {
            let reader = BufReader::new(stderr);
            for line in reader.lines().map_while(Result::ok) {
                if let Some((_stage, pct)) = parse_progress(&line) {
                    let _ = app_handle.emit(
                        "rembg-item-progress",
                        serde_json::json!({ "idx": idx, "pct": pct }),
                    );
                } else {
                    tracing::info!("rembg[{}] sidecar: {}", idx, line);
                }
            }
        });
    }

    // Drain stdout so the pipe can't fill up (sidecar prints nothing here
    // today, but keep this defensive for future metadata frames).
    if let Some(stdout) = child.stdout.take() {
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                tracing::debug!("rembg stdout: {}", line);
            }
        });
    }

    let status = child.wait().map_err(|e| e.to_string())?;
    if !status.success() {
        return Err(format!("rembg sidecar exited with {:?}", status.code()));
    }
    Ok(())
}

/// "stage=infer pct=42" → ("infer", 42)
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

/// Output path resolution honouring the "single → next to source,
/// many → sub-folder" rule per parent directory:
/// - count occurrences of each parent across the batch
/// - if a parent has only one input → output is `<parent>/<stem>_rembg.png`
/// - if a parent has 2+            → output is `<parent>/nsay_rembg/<stem>.png`
///
/// Returns paths in the same order as `inputs`.
fn derive_output_paths(inputs: &[String]) -> Vec<PathBuf> {
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
                parent.join(format!("{}_rembg.png", stem))
            } else {
                parent.join("nsay_rembg").join(format!("{}.png", stem))
            }
        })
        .collect()
}
