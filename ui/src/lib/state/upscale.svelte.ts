// Global upscale state. Mirrors rembg.svelte.ts shape — see that file
// for the architectural rationale (route-switch persistence + single
// listener subscription per app lifetime).

import { invoke, convertFileSrc } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

export type Status = "pending" | "running" | "done" | "error";
export type Item = {
  idx: number;
  input: string;
  inputUrl: string;
  output: string | null;
  outputUrl: string | null;
  bust: number;
  status: Status;
  pct: number;
  error: string | null;
};

export type Scale = 1.5 | 2 | 3 | 4;
export type ModelInfo = {
  id: string;
  label: string;
  size_mb: number;
  installed: boolean;
  output_scale: number;
};

export const upscaleStore = $state({
  items: [] as Item[],
  busy: false,
  scale: 4 as Scale,
  model: "real-esrgan-x4",
  models: [] as ModelInfo[],
  dl: null as { id: string; label: string; pct: number } | null,
  errorMsg: "",
  showInput: {} as Record<number, boolean>,
});

let initialized = false;
let nextIdx = 0;

export async function upscaleRefreshModels() {
  try {
    const all = await invoke<Array<ModelInfo & { family: string }>>("list_models");
    upscaleStore.models = all
      .filter((m) => m.family === "upscale")
      .map(({ id, label, size_mb, installed, output_scale }) => ({
        id, label, size_mb, installed, output_scale,
      }));
    if (!upscaleStore.models.find((m) => m.id === upscaleStore.model)) {
      upscaleStore.model = upscaleStore.models[0]?.id ?? "real-esrgan-x4";
    }
  } catch {
    // model picker stays at last known good selection if list_models fails
  }
}

export function upscaleInit() {
  if (initialized) return;
  initialized = true;
  upscaleRefreshModels();
  // Refresh after any download finishes so newly-installed models flip to
  // `installed: true` in the dropdown.
  listen<{ id: string }>("model-download-done", () => upscaleRefreshModels());

  listen<{ idx: number; input: string; output: string }>("upscale-item-start", (e) => {
    patch(e.payload.idx, { status: "running", pct: 0, output: e.payload.output });
  });
  listen<{ idx: number; pct: number }>("upscale-item-progress", (e) => {
    patch(e.payload.idx, { pct: e.payload.pct });
  });
  listen<{ idx: number; output: string }>("upscale-item-done", (e) => {
    patch(e.payload.idx, {
      status: "done",
      pct: 100,
      output: e.payload.output,
      outputUrl: convertFileSrc(e.payload.output),
      bust: Date.now(),
    });
  });
  listen<{ idx: number; error: string }>("upscale-item-error", (e) => {
    patch(e.payload.idx, { status: "error", error: e.payload.error });
  });

  // model-download-* events are global; both rembg and upscale stores
  // listen so whoever is on screen can show the download UI. They never
  // collide because the model id targets one tool's catalogue entry.
  listen<{ id: string; label: string }>("model-download-start", (e) => {
    upscaleStore.dl = { id: e.payload.id, label: e.payload.label, pct: 0 };
  });
  listen<{ id: string; pct: number }>("model-download-progress", (e) => {
    if (upscaleStore.dl && upscaleStore.dl.id === e.payload.id) upscaleStore.dl.pct = e.payload.pct;
  });
  listen<{ id: string }>("model-download-done", () => {
    upscaleStore.dl = null;
  });
  listen<{ id: string; error: string }>("model-download-error", (e) => {
    upscaleStore.errorMsg = `Скачивание модели: ${e.payload.error}`;
    upscaleStore.dl = null;
  });
}

function patch(idx: number, p: Partial<Item>) {
  upscaleStore.items = upscaleStore.items.map((it) => (it.idx === idx ? { ...it, ...p } : it));
}

export function upscaleAddInputs(paths: string[]) {
  if (upscaleStore.busy) return;
  const fresh: Item[] = paths.map((p) => ({
    idx: nextIdx++,
    input: p,
    inputUrl: convertFileSrc(p),
    output: null,
    outputUrl: null,
    bust: 0,
    status: "pending",
    pct: 0,
    error: null,
  }));
  upscaleStore.items = [...upscaleStore.items, ...fresh];
}

export function upscaleClear() {
  if (upscaleStore.busy) return;
  upscaleStore.items = [];
  upscaleStore.showInput = {};
  upscaleStore.errorMsg = "";
}

export function upscaleRemove(idx: number) {
  if (upscaleStore.busy) return;
  upscaleStore.items = upscaleStore.items.filter((it) => it.idx !== idx);
}

export async function upscaleRun() {
  if (upscaleStore.busy || upscaleStore.items.length === 0) return;
  upscaleStore.busy = true;
  upscaleStore.errorMsg = "";
  upscaleStore.items = upscaleStore.items.map((it, i) => ({
    ...it,
    idx: i,
    status: "pending",
    pct: 0,
    output: null,
    outputUrl: null,
    error: null,
  }));
  nextIdx = upscaleStore.items.length;
  try {
    await invoke<unknown>("upscale_run", {
      inputs: upscaleStore.items.map((it) => it.input),
      scale: upscaleStore.scale,
      model: upscaleStore.model,
    });
  } catch (e) {
    upscaleStore.errorMsg = String(e);
  } finally {
    upscaleStore.busy = false;
  }
}
