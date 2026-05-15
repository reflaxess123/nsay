// Tauri commands for backend selection. Mirrors flov::state_cmd shape.

use std::sync::{Arc, Mutex};

use crate::{config, tools};

#[derive(Clone)]
pub struct AppState {
    pub backend_choice: Arc<Mutex<String>>,
    /// Backends the app saw at startup. The Settings UI shows this so we
    /// don't claim "CUDA available" when the user uninstalled the sidecar.
    pub available_backends: Vec<String>,
}

#[derive(serde::Serialize)]
pub struct BackendState {
    pub choice: String,
    pub available: Vec<String>,
}

#[tauri::command]
pub fn get_backend_state(state: tauri::State<AppState>) -> BackendState {
    BackendState {
        choice: state.backend_choice.lock().unwrap().clone(),
        // Re-scan on each call so newly built sidecars are picked up
        // without restarting the app.
        available: tools::available_backends_any(),
    }
}

#[tauri::command]
pub fn set_backend_choice(
    choice: String,
    state: tauri::State<AppState>,
) -> Result<(), String> {
    // "docker" is vidsr-only — explicit choice for the FlashVSR-Pro container.
    // It is NOT in BACKEND_PRIORITY so "auto" never picks it; the user has
    // to opt in deliberately (Docker Desktop is a heavy dependency to assume).
    let valid = ["auto", "cuda", "dml", "vulkan", "coreml", "cpu", "docker"];
    if !valid.contains(&choice.as_str()) {
        return Err(format!("unknown backend: {}", choice));
    }
    *state.backend_choice.lock().unwrap() = choice.clone();
    config::Config::write_backend_choice(&choice).map_err(|e| e.to_string())?;
    tracing::info!("backend choice → {}", choice);
    Ok(())
}
