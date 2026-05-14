// nsay.toml parser. Surgical writes via toml_edit so user comments and
// field order survive (same trick as flov::config).

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

const CONFIG_NAME: &str = "nsay.toml";

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Config {
    #[serde(default)]
    pub backend: BackendCfg,
    #[serde(default)]
    pub models: ModelsCfg,
    #[serde(default)]
    pub rembg: ToolCfg,
    #[serde(default)]
    pub upscale: ToolCfg,
    #[serde(default)]
    pub interp: ToolCfg,
    #[serde(default)]
    pub ui: UiCfg,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct BackendCfg {
    pub choice: String,
}
impl Default for BackendCfg {
    fn default() -> Self {
        Self { choice: "auto".into() }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ModelsCfg {
    pub dir: PathBuf,
}
impl Default for ModelsCfg {
    fn default() -> Self {
        Self { dir: PathBuf::from("models") }
    }
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct ToolCfg {
    #[serde(default)]
    pub model: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct UiCfg {
    pub theme: String,
}
impl Default for UiCfg {
    fn default() -> Self {
        Self { theme: "system".into() }
    }
}

impl Config {
    /// Loads nsay.toml from the exe directory. Falls back to defaults if
    /// the file doesn't exist (first run).
    pub fn load() -> Result<Self> {
        let path = config_path()?;
        if !path.exists() {
            tracing::info!("no {} found, using defaults", CONFIG_NAME);
            return Ok(Self::default_with_files());
        }
        let raw = std::fs::read_to_string(&path)
            .with_context(|| format!("failed to read {}", path.display()))?;
        let cfg: Self = toml::from_str(&raw)
            .with_context(|| format!("failed to parse {}", path.display()))?;
        Ok(cfg)
    }

    fn default_with_files() -> Self {
        Self {
            backend: BackendCfg::default(),
            models: ModelsCfg::default(),
            rembg: ToolCfg { model: "bria-rmbg-1.4".into() },
            upscale: ToolCfg { model: "real-esrgan-x4plus".into() },
            interp: ToolCfg { model: String::new() },
            ui: UiCfg::default(),
        }
    }

    /// Surgically rewrite [backend].choice without losing comments.
    pub fn write_backend_choice(choice: &str) -> Result<()> {
        let path = config_path()?;
        let raw = if path.exists() {
            std::fs::read_to_string(&path).unwrap_or_default()
        } else {
            String::new()
        };
        let mut doc: toml_edit::DocumentMut = raw.parse().unwrap_or_default();
        let backend = doc.entry("backend").or_insert(toml_edit::table());
        if let Some(t) = backend.as_table_mut() {
            t["choice"] = toml_edit::value(choice);
        }
        std::fs::write(&path, doc.to_string())
            .with_context(|| format!("failed to write {}", path.display()))?;
        Ok(())
    }
}

fn config_path() -> Result<PathBuf> {
    let exe = std::env::current_exe().context("current_exe failed")?;
    let dir = exe.parent().context("exe has no parent")?;
    Ok(dir.join(CONFIG_NAME))
}

impl Default for Config {
    fn default() -> Self {
        Self::default_with_files()
    }
}
