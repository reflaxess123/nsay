// Global state for the video upscale page. Single-file pipeline (one
// in/out video at a time) — different shape from the rembg/upscale image
// stores which queue many items.

import { invoke, convertFileSrc } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

export type Scale = 1.5 | 2 | 3 | 4;
export type Status = "idle" | "running" | "done" | "error";

export const vidUpscaleStore = $state({
  input: "" as string,
  inputUrl: "" as string,
  output: "" as string,
  outputUrl: "" as string,
  bust: 0,
  status: "idle" as Status,
  scale: 2 as Scale,
  model: "real-esrgan-x4",
  // probe data set on vid-start
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

export function vidUpscaleInit() {
  if (initialized) return;
  initialized = true;

  // Namespaced per-tool events — see src-tauri/src/tools/video.rs.
  // Used to be a shared `vid-*` channel that both stores listened to,
  // which cross-talked progress between vidUpscale and vidSlow runs.
  listen<{
    src_w: number; src_h: number; out_w: number; out_h: number;
    fps_num: number; fps_den: number; total_frames: number;
    backend: string; encoder: string;
  }>("vid-upscale-start", (e) => {
    vidUpscaleStore.probe = e.payload;
    vidUpscaleStore.pct = 0;
    vidUpscaleStore.frame = 0;
  });
  listen<{ frame: number; total: number; pct: number }>("vid-upscale-progress", (e) => {
    vidUpscaleStore.frame = e.payload.frame;
    vidUpscaleStore.pct = e.payload.pct;
  });
  listen<{ output: string }>("vid-upscale-done", (e) => {
    vidUpscaleStore.output = e.payload.output;
    vidUpscaleStore.outputUrl = convertFileSrc(e.payload.output);
    vidUpscaleStore.bust = Date.now();
    vidUpscaleStore.status = "done";
    vidUpscaleStore.pct = 100;
  });
}

export function vidUpscaleSetInput(path: string) {
  if (vidUpscaleStore.status === "running") return;
  vidUpscaleStore.input = path;
  vidUpscaleStore.inputUrl = convertFileSrc(path);
  vidUpscaleStore.output = "";
  vidUpscaleStore.outputUrl = "";
  vidUpscaleStore.probe = null;
  vidUpscaleStore.pct = 0;
  vidUpscaleStore.frame = 0;
  vidUpscaleStore.status = "idle";
  vidUpscaleStore.errorMsg = "";
}

export function vidUpscaleClear() {
  if (vidUpscaleStore.status === "running") return;
  vidUpscaleSetInput("");
  vidUpscaleStore.input = "";
  vidUpscaleStore.inputUrl = "";
}

/** Build the output path next to the source: `{stem}_x{scale}.mp4`. */
function deriveOutput(input: string, scale: number): string {
  const sep = input.includes("\\") ? "\\" : "/";
  const idx = input.lastIndexOf(sep);
  const dir = idx >= 0 ? input.slice(0, idx) : ".";
  const base = idx >= 0 ? input.slice(idx + 1) : input;
  const dot = base.lastIndexOf(".");
  const stem = dot > 0 ? base.slice(0, dot) : base;
  const tag = Number.isInteger(scale) ? `${scale}` : `${scale}`.replace(".", "_");
  return `${dir}${sep}${stem}_x${tag}.mp4`;
}

export async function vidUpscaleRun() {
  if (vidUpscaleStore.status === "running" || !vidUpscaleStore.input) return;
  const out = deriveOutput(vidUpscaleStore.input, vidUpscaleStore.scale);
  vidUpscaleStore.status = "running";
  vidUpscaleStore.errorMsg = "";
  vidUpscaleStore.pct = 0;
  vidUpscaleStore.frame = 0;
  vidUpscaleStore.output = "";
  vidUpscaleStore.outputUrl = "";
  try {
    await invoke<unknown>("video_upscale_run", {
      input: vidUpscaleStore.input,
      output: out,
      scale: vidUpscaleStore.scale,
      model: vidUpscaleStore.model,
    });
  } catch (e) {
    vidUpscaleStore.status = "error";
    vidUpscaleStore.errorMsg = String(e);
  }
}
