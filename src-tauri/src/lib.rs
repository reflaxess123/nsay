// nsay — Tauri main app entry. Mirrors flov's lib.rs orchestration but with
// per-tool job runners instead of a single recording loop.

pub mod config;
pub mod ffmpeg;
pub mod models;
pub mod models_cmd;
pub mod state_cmd;
pub mod tools;

use std::sync::{Arc, Mutex};

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    if let Ok(log_file) = std::fs::File::create("nsay.log") {
        // DEBUG so sidecar stderr (line-by-line) and inference timings
        // land in the file — INFO would hide everything we need to
        // diagnose why a backend silently does nothing.
        let _ = tracing_subscriber::fmt()
            .with_writer(log_file)
            .with_ansi(false)
            .with_max_level(tracing::Level::DEBUG)
            .try_init();
    }
    tracing::info!("nsay starting (Tauri)");

    let cfg = config::Config::load().expect("config load failed");

    // Shared mutable backend choice — written by Settings → Backend, read by
    // every tool runner before spawn so a switch takes effect on the next
    // job, no restart required (mirrors flov's transcribe pattern).
    let backend_choice = Arc::new(Mutex::new(cfg.backend.choice.clone()));

    let available = tools::available_backends_any();
    tracing::info!(
        "available backends (any tool): {:?}; configured choice: {}",
        available,
        cfg.backend.choice
    );

    let app_state = state_cmd::AppState {
        backend_choice: backend_choice.clone(),
        available_backends: available.clone(),
    };

    let model_state = models_cmd::ModelState {
        models_dir: Arc::new(Mutex::new(cfg.models.dir.clone())),
        in_flight: Arc::new(Mutex::new(Vec::new())),
    };

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .manage(app_state)
        .manage(model_state)
        .invoke_handler(tauri::generate_handler![
            // Backend / settings
            state_cmd::get_backend_state,
            state_cmd::set_backend_choice,
            // Models
            models_cmd::list_models,
            models_cmd::download_model,
            models_cmd::delete_model,
            // Tools
            tools::rembg::rembg_run,
            tools::upscale::upscale_run,
            tools::video::video_upscale_run,
            tools::video::video_interp_run,
            tools::video::video_vidsr_run,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
