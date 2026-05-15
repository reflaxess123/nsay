// nsay-vidsr-docker — thin Rust shim that spawns `docker run flashvsr-pro:latest`.
//
// Why a Rust binary at all (instead of just `docker run` from the Tauri runner)?
// - Lives at the same path layout as other sidecars (nsay-<tool>-<backend>.exe
//   next to nsay_app.exe), so tools::resolve_sidecar finds it without a special
//   case for the docker backend.
// - Translates Windows host paths into per-volume mount args (input dir,
//   output dir, weights dir) so the FlashVSR-Pro container script sees clean
//   /in /out /weights paths — the container is Linux and can't see D:\foo.
// - Re-emits FlashVSR-Pro's tqdm progress as `frame N` lines on stderr so the
//   Tauri side's spawn_progress (parses `frame N`) drives the same UI bar as
//   the libtorch backend.
// - Locates the result mp4 inside /out after the run finishes (FlashVSR-Pro
//   chooses its own output filename) and renames it to whatever --output the
//   caller asked for. This keeps the contract identical to the libtorch path
//   from the Tauri side.
//
// Block-Sparse-Attention compile is Linux-only (mit-han-lab/Block-Sparse-Attention
// README: "Linux."), so this shim is the only realistic Windows path. The
// container handles GPU access via Docker Desktop's WSL2 backend with the
// NVIDIA Container Toolkit; --gpus all is the trigger.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{anyhow, bail, Context, Result};
use regex::Regex;

const DEFAULT_IMAGE: &str = "flashvsr-pro:latest";
const CONTAINER_INPUT_DIR: &str = "/in";
const CONTAINER_OUTPUT_DIR: &str = "/out";
const CONTAINER_WEIGHTS_DIR: &str = "/workspace/FlashVSR-Pro/models/FlashVSR-v1.1";

#[derive(Debug)]
struct Args {
    input: PathBuf,
    output: PathBuf,
    scale: u32,
    mode: String,           // "tiny" | "full"
    tile_vae: bool,
    tile_dit: bool,
    keep_audio: bool,
    weights_dir: PathBuf,   // host path, mounted read-only
    image: String,
}

fn parse_args() -> Result<Args> {
    let mut input: Option<PathBuf> = None;
    let mut output: Option<PathBuf> = None;
    let mut scale: u32 = 4;
    let mut mode = String::from("tiny");
    let mut tile_vae = true;
    let mut tile_dit = true;
    let mut keep_audio = true;
    let mut weights_dir: Option<PathBuf> = None;
    let mut image = String::from(DEFAULT_IMAGE);

    let raw: Vec<String> = std::env::args().skip(1).collect();
    let mut i = 0usize;
    while i < raw.len() {
        let a = raw[i].as_str();
        match a {
            "--input" | "-i" => {
                input = Some(PathBuf::from(need_val(&raw, &mut i, a)?));
            }
            "--output" | "-o" => {
                output = Some(PathBuf::from(need_val(&raw, &mut i, a)?));
            }
            "--scale" => {
                scale = need_val(&raw, &mut i, a)?
                    .parse()
                    .with_context(|| format!("--scale must be int, got {a}"))?;
            }
            "--mode" => {
                mode = need_val(&raw, &mut i, a)?;
                if mode != "tiny" && mode != "full" {
                    bail!("--mode must be 'tiny' or 'full', got '{}'", mode);
                }
            }
            "--tile-vae"        => { tile_vae = true; }
            "--no-tile-vae"     => { tile_vae = false; }
            "--tile-dit"        => { tile_dit = true; }
            "--no-tile-dit"     => { tile_dit = false; }
            "--keep-audio"      => { keep_audio = true; }
            "--no-keep-audio"   => { keep_audio = false; }
            "--weights-dir" => {
                weights_dir = Some(PathBuf::from(need_val(&raw, &mut i, a)?));
            }
            "--image" => {
                image = need_val(&raw, &mut i, a)?;
            }
            // Tauri runner currently passes --model <path>; ignore so callers
            // can stay generic. The model is encoded in the docker image, not
            // a file on disk.
            "--model" => { let _ = need_val(&raw, &mut i, a)?; }
            other => bail!("unknown argument: {}", other),
        }
        i += 1;
    }

    let input = input.ok_or_else(|| anyhow!("--input is required"))?;
    let output = output.ok_or_else(|| anyhow!("--output is required"))?;
    let weights_dir = weights_dir
        .or_else(default_weights_dir)
        .ok_or_else(|| anyhow!("--weights-dir not given and APPDATA not set"))?;

    Ok(Args { input, output, scale, mode, tile_vae, tile_dit, keep_audio, weights_dir, image })
}

fn need_val(raw: &[String], i: &mut usize, flag: &str) -> Result<String> {
    *i += 1;
    raw.get(*i)
        .cloned()
        .ok_or_else(|| anyhow!("flag {} expects a value", flag))
}

fn default_weights_dir() -> Option<PathBuf> {
    // Same convention as src-tauri/config.rs default models dir:
    // %APPDATA%/nsay/models/flashvsr-v1.1
    std::env::var_os("APPDATA")
        .map(|s| PathBuf::from(s).join("nsay").join("models").join("flashvsr-v1.1"))
}

fn find_docker() -> Result<PathBuf> {
    let bin = if cfg!(target_os = "windows") { "docker.exe" } else { "docker" };
    let paths = std::env::var_os("PATH").context("PATH not set")?;
    for dir in std::env::split_paths(&paths) {
        let candidate = dir.join(bin);
        if candidate.exists() {
            return Ok(candidate);
        }
    }
    bail!(
        "{} not found in PATH. Install Docker Desktop and ensure WSL2 + \
         NVIDIA Container Toolkit are enabled.",
        bin
    )
}

fn ensure_image(docker: &Path, image: &str) -> Result<()> {
    let out = Command::new(docker)
        .args(["images", "-q", image])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .context("failed to invoke `docker images`")?;
    if !out.status.success() {
        bail!(
            "`docker images -q {}` failed (exit {:?}). Is Docker Desktop running?",
            image, out.status.code()
        );
    }
    let id = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if id.is_empty() {
        bail!(
            "image '{}' not found locally. Run scripts/setup-flashvsr-docker.ps1 \
             to build it (one-time, ~30 minutes).",
            image
        );
    }
    Ok(())
}

fn ensure_weights(weights_dir: &Path) -> Result<()> {
    if !weights_dir.is_dir() {
        bail!(
            "weights dir not found: {}\n\
             Run scripts/setup-flashvsr-docker.ps1 to download the FlashVSR-v1.1 \
             checkpoints from HuggingFace.",
            weights_dir.display()
        );
    }
    Ok(())
}

fn list_mp4(dir: &Path) -> Result<HashSet<PathBuf>> {
    let mut set = HashSet::new();
    if !dir.exists() {
        std::fs::create_dir_all(dir)
            .with_context(|| format!("failed to create output dir: {}", dir.display()))?;
        return Ok(set);
    }
    for entry in std::fs::read_dir(dir)
        .with_context(|| format!("read_dir({})", dir.display()))?
    {
        let p = entry?.path();
        if p.extension().and_then(|e| e.to_str()).map(|s| s.eq_ignore_ascii_case("mp4")) == Some(true) {
            set.insert(p);
        }
    }
    Ok(set)
}

/// Convert "name with spaces.mp4" → safe basename for the container side
/// (avoid shell quoting issues in args we hand to `docker run` argv).
fn container_basename(p: &Path) -> Result<String> {
    p.file_name()
        .and_then(|s| s.to_str())
        .map(|s| s.to_string())
        .ok_or_else(|| anyhow!("path has no filename: {}", p.display()))
}

fn run() -> Result<()> {
    let args = parse_args()?;
    let docker = find_docker()?;
    ensure_image(&docker, &args.image)?;
    ensure_weights(&args.weights_dir)?;

    let input_abs = std::fs::canonicalize(&args.input)
        .with_context(|| format!("input not found: {}", args.input.display()))?;
    let input_dir = input_abs
        .parent()
        .ok_or_else(|| anyhow!("input has no parent: {}", input_abs.display()))?
        .to_path_buf();
    let input_name = container_basename(&input_abs)?;

    let output_abs = if args.output.is_absolute() {
        args.output.clone()
    } else {
        std::env::current_dir()?.join(&args.output)
    };
    let output_dir = output_abs
        .parent()
        .ok_or_else(|| anyhow!("output has no parent: {}", output_abs.display()))?
        .to_path_buf();
    std::fs::create_dir_all(&output_dir)
        .with_context(|| format!("create output dir: {}", output_dir.display()))?;

    let weights_abs = std::fs::canonicalize(&args.weights_dir)
        .with_context(|| format!("weights dir: {}", args.weights_dir.display()))?;

    let pre_existing = list_mp4(&output_dir)?;

    eprintln!(
        "nsay-vidsr-docker: image={} mode={} scale=x{} input={}",
        args.image, args.mode, args.scale, input_abs.display()
    );

    // Build docker run command.
    //
    // Mount layout:
    //   <input_dir>     → /in   (ro)
    //   <output_dir>    → /out  (rw, FlashVSR-Pro writes its result here)
    //   <weights_dir>   → /workspace/FlashVSR-Pro/models/FlashVSR-v1.1 (ro)
    //
    // --gpus all needs Docker Desktop with the NVIDIA Container Toolkit
    // wired into WSL2; if missing, docker prints a clear error and exits 125.
    //
    // --shm-size=8g — PyTorch DataLoader workers share memory through /dev/shm
    // and the default 64MB causes "bus error" crashes on inference >1080p.
    let mut cmd = Command::new(&docker);
    cmd.arg("run")
        .arg("--rm")
        .arg("--gpus").arg("all")
        .arg("--shm-size=8g")
        .arg("-v").arg(format!("{}:{}:ro", input_dir.display(), CONTAINER_INPUT_DIR))
        .arg("-v").arg(format!("{}:{}",    output_dir.display(), CONTAINER_OUTPUT_DIR))
        .arg("-v").arg(format!("{}:{}:ro", weights_abs.display(), CONTAINER_WEIGHTS_DIR))
        .arg(&args.image)
        // Container entrypoint — FlashVSR-Pro's unified script.
        .arg("python").arg("infer.py")
        .arg("-i").arg(format!("{}/{}", CONTAINER_INPUT_DIR, input_name))
        .arg("-o").arg(format!("{}/", CONTAINER_OUTPUT_DIR))
        .arg("--mode").arg(&args.mode)
        .arg("--scale").arg(args.scale.to_string());
    if args.tile_vae   { cmd.arg("--tile-vae"); }
    if args.tile_dit   { cmd.arg("--tile-dit"); }
    if args.keep_audio { cmd.arg("--keep-audio"); }
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut child = cmd.spawn().context("failed to spawn `docker run`")?;
    let stderr = child.stderr.take().expect("piped stderr");
    let stdout = child.stdout.take().expect("piped stdout");

    // tqdm writes carriage-return-terminated progress to stderr. Some setups
    // route to stdout — read both, parse same way. Frame events go to OUR
    // stderr as `frame N` so the Tauri runner's spawn_progress (which already
    // looks for that prefix from libtorch backends) drives the same bar.
    let stderr_thread = std::thread::spawn(move || pump_progress("stderr", stderr));
    let stdout_thread = std::thread::spawn(move || pump_progress("stdout", stdout));

    let status = child.wait().context("docker wait failed")?;
    let _ = stderr_thread.join();
    let _ = stdout_thread.join();

    if !status.success() {
        bail!("docker exited with code {:?}", status.code());
    }

    // Find the new mp4 the container produced and rename it to args.output.
    // FlashVSR-Pro picks its own filename based on the input; we don't try
    // to predict it.
    let after = list_mp4(&output_dir)?;
    let new_files: Vec<&PathBuf> = after.difference(&pre_existing).collect();
    let produced = match new_files.as_slice() {
        []      => bail!("docker run finished but no new mp4 appeared in {}", output_dir.display()),
        [one]   => (*one).clone(),
        many    => bail!("expected exactly one new mp4 in {}, found {}: {:?}",
                         output_dir.display(), many.len(), many),
    };
    if produced != output_abs {
        std::fs::rename(&produced, &output_abs)
            .with_context(|| format!("rename {} → {}", produced.display(), output_abs.display()))?;
    }

    eprintln!("nsay-vidsr-docker: done → {}", output_abs.display());
    Ok(())
}

/// Read child output line-by-line, translate tqdm percentage bars into
/// `frame N` lines on our own stderr, log everything else verbatim.
///
/// tqdm format example: `90%|████▌| 90/100 [00:09<00:01, 9.81it/s]`
/// — we capture the first `\d+/\d+` pair and emit "frame {N}" once per
/// integer increment so spawn_progress on the Tauri side advances the bar.
fn pump_progress(label: &str, src: impl std::io::Read + Send + 'static) {
    use std::io::BufReader;
    // tqdm separates updates with \r, not \n — wrap a byte-level reader that
    // splits on either so each progress redraw is one "line" we can parse.
    let mut reader = BufReader::new(src);
    let re = Regex::new(r"(\d+)\s*/\s*(\d+)").expect("regex");
    let mut last_emitted: u64 = 0;
    let mut buf: Vec<u8> = Vec::with_capacity(256);
    loop {
        buf.clear();
        let n = match read_until_lf_or_cr(&mut reader, &mut buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(_) => break,
        };
        let _ = n;
        let line = String::from_utf8_lossy(&buf);
        let trimmed = line.trim();
        if trimmed.is_empty() { continue; }

        if let Some(caps) = re.captures(trimmed) {
            if let (Ok(cur), Ok(_total)) = (
                caps.get(1).unwrap().as_str().parse::<u64>(),
                caps.get(2).unwrap().as_str().parse::<u64>(),
            ) {
                if cur > last_emitted {
                    last_emitted = cur;
                    eprintln!("frame {}", cur);
                    continue;
                }
            }
        }
        eprintln!("{}: {}", label, trimmed);
    }
}

/// Like read_until('\n') but treats either '\n' or '\r' as a record sep so
/// tqdm's carriage-return updates each become a parseable record.
fn read_until_lf_or_cr<R: std::io::BufRead>(r: &mut R, buf: &mut Vec<u8>) -> std::io::Result<usize> {
    let mut total = 0usize;
    loop {
        let available = match r.fill_buf() {
            Ok(b) if b.is_empty() => return Ok(total),
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        };
        if let Some(idx) = available.iter().position(|&c| c == b'\n' || c == b'\r') {
            buf.extend_from_slice(&available[..idx]);
            total += idx + 1;
            r.consume(idx + 1);
            return Ok(total);
        }
        let len = available.len();
        buf.extend_from_slice(available);
        total += len;
        r.consume(len);
    }
}

fn main() {
    if let Err(e) = run() {
        eprintln!("nsay-vidsr-docker error: {e:#}");
        std::process::exit(1);
    }
}
