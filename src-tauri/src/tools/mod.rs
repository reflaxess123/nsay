// Sidecar resolution + spawn helpers. Each tool family (rembg / upscale /
// interp) ships a binary per backend named `nsay-<tool>-<backend>(.exe)`,
// staged next to nsay_app.exe by scripts/build-sidecars.ps1.

pub mod rembg;
pub mod upscale;
pub mod video;

use anyhow::{Context, Result};
use std::path::PathBuf;

/// Order tried when the user picked "auto". First sidecar that exists wins.
/// Mirrors ui/src/lib/settings/Backend.svelte.
pub const BACKEND_PRIORITY: &[&str] = &["cuda", "dml", "vulkan", "coreml", "cpu"];

pub const TOOLS: &[&str] = &["rembg", "upscale", "interp", "vidsr"];

pub fn sidecar_bin_name(tool: &str, backend: &str) -> String {
    if cfg!(target_os = "windows") {
        format!("nsay-{}-{}.exe", tool, backend)
    } else {
        format!("nsay-{}-{}", tool, backend)
    }
}

pub fn exe_dir() -> Result<PathBuf> {
    let exe = std::env::current_exe().context("current_exe failed")?;
    Ok(exe.parent().context("exe has no parent")?.to_path_buf())
}

/// Backends with at least one sidecar present (any tool). Used by Settings
/// to grey-out unavailable choices.
pub fn available_backends_any() -> Vec<String> {
    let dir = match exe_dir() {
        Ok(d) => d,
        Err(_) => return Vec::new(),
    };
    BACKEND_PRIORITY
        .iter()
        .filter(|b| {
            TOOLS
                .iter()
                .any(|t| dir.join(sidecar_bin_name(t, b)).exists())
        })
        .map(|b| (*b).to_string())
        .collect()
}

/// Resolve which sidecar binary to spawn for a given tool + user choice.
/// - NSAY_BACKEND env var wins (debug knob)
/// - "auto" walks BACKEND_PRIORITY for this specific tool
/// - explicit name (cuda/dml/...) errors if missing
pub fn resolve_sidecar(tool: &str, choice: &str) -> Result<(String, PathBuf)> {
    let dir = exe_dir()?;
    let effective = std::env::var("NSAY_BACKEND").unwrap_or_else(|_| choice.to_string());

    if effective != "auto" {
        let candidate = dir.join(sidecar_bin_name(tool, &effective));
        if !candidate.exists() {
            anyhow::bail!(
                "backend '{}' selected for tool '{}' but {:?} not found",
                effective,
                tool,
                candidate
            );
        }
        return Ok((effective, candidate));
    }

    let mut tried = Vec::new();
    for backend in BACKEND_PRIORITY {
        let candidate = dir.join(sidecar_bin_name(tool, backend));
        if candidate.exists() {
            return Ok(((*backend).to_string(), candidate));
        }
        tried.push(candidate);
    }
    anyhow::bail!(
        "no sidecar for tool '{}' found in {:?}; tried {:?}",
        tool,
        dir,
        tried
    );
}
