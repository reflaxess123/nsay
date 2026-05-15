// Tauri commands for model management: list catalogue + download status,
// download with progress events, delete.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use crate::models;

#[derive(Clone)]
pub struct ModelState {
    pub models_dir: Arc<Mutex<PathBuf>>,
    /// IDs currently downloading — UI greys these out.
    pub in_flight: Arc<Mutex<Vec<String>>>,
}

#[derive(serde::Serialize)]
pub struct ModelInfo {
    pub id: String,
    pub family: String,
    pub label: String,
    pub size_mb: u32,
    pub installed: bool,
    pub downloading: bool,
    pub local_path: Option<String>,
    /// Catalogue field; 0 for non-upscale entries. UI uses it to gate the
    /// scale toggle against the model's native ratio.
    pub output_scale: u32,
}

/// Pub so tools::rembg / future tools share the same resolution logic.
/// Order:
///   1. NSAY_MODELS_DIR env var (dev knob; absolute path).
///   2. ModelState's configured `dir` if absolute.
///   3. `<user-data>/nsay/models/` — default. Same place a packaged build
///      uses, so dev and prod look at the same files unless you override.
///      User-data resolves to %APPDATA% on Windows, ~/Library/Application
///      Support on macOS, ~/.local/share on Linux.
pub fn resolve_models_dir(state: &ModelState) -> PathBuf {
    if let Ok(env_dir) = std::env::var("NSAY_MODELS_DIR") {
        return PathBuf::from(env_dir);
    }
    let dir = state.models_dir.lock().unwrap().clone();
    if dir.is_absolute() {
        return dir;
    }
    // Relative `dir` (default "models") gets joined under the user data root.
    let base = dirs::data_dir().unwrap_or_else(|| PathBuf::from("."));
    base.join("nsay").join(&dir)
}

#[tauri::command]
pub fn list_models(state: tauri::State<ModelState>) -> Vec<ModelInfo> {
    let dir = resolve_models_dir(&state);
    let in_flight = state.in_flight.lock().unwrap().clone();
    models::CATALOG
        .iter()
        .map(|m| {
            let path = dir.join(m.filename);
            let installed = path.exists();
            ModelInfo {
                id: m.id.to_string(),
                family: m.family.to_string(),
                label: m.label.to_string(),
                size_mb: m.size_mb,
                installed,
                downloading: in_flight.contains(&m.id.to_string()),
                local_path: installed.then(|| path.to_string_lossy().into_owned()),
                output_scale: m.output_scale,
            }
        })
        .collect()
}

#[tauri::command]
pub fn download_model(
    id: String,
    state: tauri::State<ModelState>,
    app: tauri::AppHandle,
) -> Result<(), String> {
    let entry = models::find(&id).ok_or_else(|| format!("unknown model id: {}", id))?;
    let dir = resolve_models_dir(&state);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let dest = dir.join(entry.filename);
    if dest.exists() {
        return Ok(());
    }

    {
        let mut in_flight = state.in_flight.lock().unwrap();
        if in_flight.contains(&id) {
            return Err("already downloading".into());
        }
        in_flight.push(id.clone());
    }

    let in_flight_handle = state.in_flight.clone();
    let url = entry.url.to_string();
    let id_for_thread = id.clone();
    let dest_for_thread = dest.clone();

    std::thread::spawn(move || {
        let result = download_to(&url, &dest_for_thread, &app, &id_for_thread);

        let mut in_flight = in_flight_handle.lock().unwrap();
        in_flight.retain(|x| x != &id_for_thread);

        if let Err(e) = result {
            tracing::error!("download {} failed: {:#}", id_for_thread, e);
            let _ = tauri::Emitter::emit(
                &app,
                "model-download-error",
                serde_json::json!({ "id": id_for_thread, "error": e.to_string() }),
            );
        } else {
            let _ = tauri::Emitter::emit(
                &app,
                "model-download-done",
                serde_json::json!({ "id": id_for_thread }),
            );
        }
    });

    Ok(())
}

/// Synchronous "make sure this model is on disk, downloading if not" used
/// by tool runners (rembg/upscale/interp) so the user doesn't have to
/// pre-fetch from Settings before pressing the action button. Emits the
/// same model-download-progress / -done / -error events as the Tauri
/// command, so the UI can show one progress UI no matter who triggered
/// the download.
pub fn ensure_model(
    id: &str,
    state: &ModelState,
    app: &tauri::AppHandle,
) -> anyhow::Result<PathBuf> {
    let entry = models::find(id)
        .ok_or_else(|| anyhow::anyhow!("unknown model id: {}", id))?;
    let dir = resolve_models_dir(state);
    std::fs::create_dir_all(&dir)?;
    let dest = dir.join(entry.filename);
    if dest.exists() {
        return Ok(dest);
    }

    // Mark in-flight so list_models / Settings UI greys it out, and so
    // we don't double-download if the user mashes the button.
    {
        let mut in_flight = state.in_flight.lock().unwrap();
        if in_flight.contains(&id.to_string()) {
            anyhow::bail!("model {} is already downloading", id);
        }
        in_flight.push(id.to_string());
    }

    let _ = tauri::Emitter::emit(
        app,
        "model-download-start",
        serde_json::json!({ "id": id, "label": entry.label, "size_mb": entry.size_mb }),
    );

    let result = download_to(entry.url, &dest, app, id);

    // Pop in-flight BEFORE emitting done/error so a listener that calls
    // list_models in response sees the up-to-date state.
    {
        let mut in_flight = state.in_flight.lock().unwrap();
        in_flight.retain(|x| x != id);
    }

    match result {
        Ok(()) => {
            let _ = tauri::Emitter::emit(
                app,
                "model-download-done",
                serde_json::json!({ "id": id }),
            );
            Ok(dest)
        }
        Err(e) => {
            tracing::error!("ensure_model {} failed: {:#}", id, e);
            let _ = tauri::Emitter::emit(
                app,
                "model-download-error",
                serde_json::json!({ "id": id, "error": e.to_string() }),
            );
            Err(e)
        }
    }
}

fn download_to(
    url: &str,
    dest: &std::path::Path,
    app: &tauri::AppHandle,
    id: &str,
) -> anyhow::Result<()> {
    use std::io::Read;
    tracing::info!("downloading {} → {}", url, dest.display());
    let response = ureq::get(url).call()?;

    let total = response
        .headers()
        .get("content-length")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(0);

    let tmp = dest.with_extension("part");
    let mut file = std::fs::File::create(&tmp)?;
    let mut reader = response.into_body().into_reader();
    let mut buf = [0u8; 64 * 1024];
    let mut downloaded: u64 = 0;
    let mut last_pct = 0u8;

    loop {
        let n = reader.read(&mut buf)?;
        if n == 0 {
            break;
        }
        std::io::Write::write_all(&mut file, &buf[..n])?;
        downloaded += n as u64;
        if total > 0 {
            let pct = ((downloaded * 100) / total) as u8;
            if pct != last_pct {
                last_pct = pct;
                let _ = tauri::Emitter::emit(
                    app,
                    "model-download-progress",
                    serde_json::json!({ "id": id, "pct": pct }),
                );
            }
        }
    }

    drop(file);
    std::fs::rename(&tmp, dest)?;
    Ok(())
}

#[tauri::command]
pub fn delete_model(id: String, state: tauri::State<ModelState>) -> Result<(), String> {
    let entry = models::find(&id).ok_or_else(|| format!("unknown model id: {}", id))?;
    let dir = resolve_models_dir(&state);
    let path = dir.join(entry.filename);
    if path.exists() {
        std::fs::remove_file(&path).map_err(|e| e.to_string())?;
    }
    Ok(())
}
