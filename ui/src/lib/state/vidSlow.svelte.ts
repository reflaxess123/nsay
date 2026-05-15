// Global state for the slow-motion (RIFE frame interpolation) page.
// Same single-file shape as vidUpscale.

import { invoke, convertFileSrc } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

export type Factor = 2 | 4 | 8;
export type Mode = "boost" | "slow";
export type Status = "idle" | "running" | "done" | "error";

export const vidSlowStore = $state({
  input: "" as string,
  inputUrl: "" as string,
  output: "" as string,
  outputUrl: "" as string,
  bust: 0,
  status: "idle" as Status,
  factor: 2 as Factor,
  mode: "boost" as Mode,
  model: "rife-4.9",
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

export function vidSlowInit() {
  if (initialized) return;
  initialized = true;

  // Namespaced per-tool events — see src-tauri/src/tools/video.rs.
  // The status guards are belt-and-suspenders now that events are
  // tool-specific; before namespacing they were the only thing
  // preventing vidUpscale runs from updating this store.
  listen<{
    src_w: number; src_h: number; out_w: number; out_h: number;
    fps_num: number; fps_den: number; total_frames: number;
    backend: string; encoder: string;
  }>("vid-interp-start", (e) => {
    if (vidSlowStore.status !== "running") return;
    vidSlowStore.probe = e.payload;
    vidSlowStore.pct = 0;
    vidSlowStore.frame = 0;
  });
  listen<{ frame: number; total: number; pct: number }>("vid-interp-progress", (e) => {
    if (vidSlowStore.status !== "running") return;
    vidSlowStore.frame = e.payload.frame;
    vidSlowStore.pct = e.payload.pct;
  });
  listen<{ output: string }>("vid-interp-done", (e) => {
    if (vidSlowStore.status !== "running") return;
    vidSlowStore.output = e.payload.output;
    vidSlowStore.outputUrl = convertFileSrc(e.payload.output);
    vidSlowStore.bust = Date.now();
    vidSlowStore.status = "done";
    vidSlowStore.pct = 100;
  });
}

export function vidSlowSetInput(path: string) {
  if (vidSlowStore.status === "running") return;
  vidSlowStore.input = path;
  vidSlowStore.inputUrl = convertFileSrc(path);
  vidSlowStore.output = "";
  vidSlowStore.outputUrl = "";
  vidSlowStore.probe = null;
  vidSlowStore.pct = 0;
  vidSlowStore.frame = 0;
  vidSlowStore.status = "idle";
  vidSlowStore.errorMsg = "";
}

export function vidSlowClear() {
  if (vidSlowStore.status === "running") return;
  vidSlowSetInput("");
  vidSlowStore.input = "";
  vidSlowStore.inputUrl = "";
}

function deriveOutput(input: string, factor: number, mode: Mode): string {
  const sep = input.includes("\\") ? "\\" : "/";
  const idx = input.lastIndexOf(sep);
  const dir = idx >= 0 ? input.slice(0, idx) : ".";
  const base = idx >= 0 ? input.slice(idx + 1) : input;
  const dot = base.lastIndexOf(".");
  const stem = dot > 0 ? base.slice(0, dot) : base;
  const tag = mode === "slow" ? `slow${factor}x` : `${factor}xfps`;
  return `${dir}${sep}${stem}_${tag}.mp4`;
}

export async function vidSlowRun() {
  if (vidSlowStore.status === "running" || !vidSlowStore.input) return;
  const out = deriveOutput(vidSlowStore.input, vidSlowStore.factor, vidSlowStore.mode);
  vidSlowStore.status = "running";
  vidSlowStore.errorMsg = "";
  vidSlowStore.pct = 0;
  vidSlowStore.frame = 0;
  vidSlowStore.output = "";
  vidSlowStore.outputUrl = "";
  try {
    await invoke<unknown>("video_interp_run", {
      input: vidSlowStore.input,
      output: out,
      factor: vidSlowStore.factor,
      mode: vidSlowStore.mode,
      model: vidSlowStore.model,
    });
  } catch (e) {
    vidSlowStore.status = "error";
    vidSlowStore.errorMsg = String(e);
  }
}
