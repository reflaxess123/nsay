// Global state for the temporal video super-resolution page (PLAN F4).
// Same single-file shape as vidUpscale but spawns the libtorch-backed
// nsay-vidsr-* sidecar via the video_vidsr_run Tauri command.
//
// Events are namespaced vid-vidsr-* (see src-tauri/src/tools/video.rs).

import { invoke, convertFileSrc } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

export type Scale = 2 | 3 | 4;
export type Status = "idle" | "running" | "done" | "error";
export type DiffMode = "tiny" | "full";

export const vidsrStore = $state({
  input: "" as string,
  inputUrl: "" as string,
  output: "" as string,
  outputUrl: "" as string,
  bust: 0,
  status: "idle" as Status,
  scale: 4 as Scale,
  // Clip size the sidecar feeds the model per forward pass. RealBasicVSR
  // was trained / benchmarked at 15. Lower = less RAM, more boundary
  // artifacts; higher = smoother but linearly more VRAM.
  window: 15,
  model: "realbasicvsr-x4",
  // FlashVSR-Pro (docker backend) tuning. Ignored by libtorch sidecars.
  // tiny = lighter VAE branch, fits 8 GB VRAM with tiling enabled;
  // full = stronger detail, needs >12 GB without tiling.
  mode: "tiny" as DiffMode,
  tileVae: true,
  tileDit: true,
  keepAudio: true,
  probe: null as null | {
    src_w: number; src_h: number;
    out_w: number; out_h: number;
    fps_num: number; fps_den: number;
    total_frames: number;
    backend: string; encoder: string;
  },
  pct: 0,
  frame: 0,
  errorMsg: "",
});

let initialized = false;

export function vidsrInit() {
  if (initialized) return;
  initialized = true;

  listen<{
    src_w: number; src_h: number; out_w: number; out_h: number;
    fps_num: number; fps_den: number; total_frames: number;
    backend: string; encoder: string;
  }>("vid-vidsr-start", (e) => {
    if (vidsrStore.status !== "running") return;
    vidsrStore.probe = e.payload;
    vidsrStore.pct = 0;
    vidsrStore.frame = 0;
  });
  listen<{ frame: number; total: number; pct: number }>("vid-vidsr-progress", (e) => {
    if (vidsrStore.status !== "running") return;
    vidsrStore.frame = e.payload.frame;
    vidsrStore.pct = e.payload.pct;
  });
  listen<{ output: string }>("vid-vidsr-done", (e) => {
    if (vidsrStore.status !== "running") return;
    vidsrStore.output = e.payload.output;
    vidsrStore.outputUrl = convertFileSrc(e.payload.output);
    vidsrStore.bust = Date.now();
    vidsrStore.status = "done";
    vidsrStore.pct = 100;
  });
}

export function vidsrSetInput(path: string) {
  if (vidsrStore.status === "running") return;
  vidsrStore.input = path;
  vidsrStore.inputUrl = convertFileSrc(path);
  vidsrStore.output = "";
  vidsrStore.outputUrl = "";
  vidsrStore.probe = null;
  vidsrStore.pct = 0;
  vidsrStore.frame = 0;
  vidsrStore.status = "idle";
  vidsrStore.errorMsg = "";
}

export function vidsrClear() {
  if (vidsrStore.status === "running") return;
  vidsrSetInput("");
  vidsrStore.input = "";
  vidsrStore.inputUrl = "";
}

function deriveOutput(input: string, scale: number): string {
  const sep = input.includes("\\") ? "\\" : "/";
  const idx = input.lastIndexOf(sep);
  const dir = idx >= 0 ? input.slice(0, idx) : ".";
  const base = idx >= 0 ? input.slice(idx + 1) : input;
  const dot = base.lastIndexOf(".");
  const stem = dot > 0 ? base.slice(0, dot) : base;
  return `${dir}${sep}${stem}_vsr${scale}x.mp4`;
}

export async function vidsrRun() {
  if (vidsrStore.status === "running" || !vidsrStore.input) return;
  const out = deriveOutput(vidsrStore.input, vidsrStore.scale);
  vidsrStore.status = "running";
  vidsrStore.errorMsg = "";
  vidsrStore.pct = 0;
  vidsrStore.frame = 0;
  vidsrStore.output = "";
  vidsrStore.outputUrl = "";
  try {
    await invoke<unknown>("video_vidsr_run", {
      input: vidsrStore.input,
      output: out,
      scale: vidsrStore.scale,
      window: vidsrStore.window,
      model: vidsrStore.model,
      mode: vidsrStore.mode,
      tileVae: vidsrStore.tileVae,
      tileDit: vidsrStore.tileDit,
      keepAudio: vidsrStore.keepAudio,
    });
  } catch (e) {
    vidsrStore.status = "error";
    vidsrStore.errorMsg = String(e);
  }
}
