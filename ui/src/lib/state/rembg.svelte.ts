// Global rembg state. Lives for the app's whole lifetime so switching
// routes (Rembg → Settings → Rembg) doesn't lose the queue, the in-flight
// inference, or the listener subscriptions to backend events.
//
// Page components import `rembgStore` and call `rembgInit()` once on
// mount (idempotent). All mutations go through the action functions
// below — pages never mutate the store directly so the action surface
// stays small and easy to reason about.

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

export const rembgStore = $state({
  items: [] as Item[],
  busy: false,
  model: "bria-rmbg-1.4",
  dl: null as { id: string; label: string; pct: number } | null,
  errorMsg: "",
  showInput: {} as Record<number, boolean>,
  choke: 0.0,
});

let initialized = false;
let nextIdx = 0;

/** Wire backend events into the store. Idempotent — safe to call from
 *  every page that uses rembg. */
export function rembgInit() {
  if (initialized) return;
  initialized = true;

  listen<{ idx: number; input: string; output: string }>("rembg-item-start", (e) => {
    patch(e.payload.idx, { status: "running", pct: 0, output: e.payload.output });
  });
  listen<{ idx: number; pct: number }>("rembg-item-progress", (e) => {
    patch(e.payload.idx, { pct: e.payload.pct });
  });
  listen<{ idx: number; output: string }>("rembg-item-done", (e) => {
    patch(e.payload.idx, {
      status: "done",
      pct: 100,
      output: e.payload.output,
      outputUrl: convertFileSrc(e.payload.output),
      bust: Date.now(),
    });
  });
  listen<{ idx: number; error: string }>("rembg-item-error", (e) => {
    patch(e.payload.idx, { status: "error", error: e.payload.error });
  });

  listen<{ id: string; label: string }>("model-download-start", (e) => {
    rembgStore.dl = { id: e.payload.id, label: e.payload.label, pct: 0 };
  });
  listen<{ id: string; pct: number }>("model-download-progress", (e) => {
    if (rembgStore.dl && rembgStore.dl.id === e.payload.id) rembgStore.dl.pct = e.payload.pct;
  });
  listen<{ id: string }>("model-download-done", () => {
    rembgStore.dl = null;
  });
  listen<{ id: string; error: string }>("model-download-error", (e) => {
    rembgStore.errorMsg = `Скачивание модели: ${e.payload.error}`;
    rembgStore.dl = null;
  });
}

function patch(idx: number, p: Partial<Item>) {
  rembgStore.items = rembgStore.items.map((it) => (it.idx === idx ? { ...it, ...p } : it));
}

export function rembgAddInputs(paths: string[]) {
  if (rembgStore.busy) return;
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
  rembgStore.items = [...rembgStore.items, ...fresh];
}

export function rembgClear() {
  if (rembgStore.busy) return;
  rembgStore.items = [];
  rembgStore.showInput = {};
  rembgStore.errorMsg = "";
}

export function rembgRemove(idx: number) {
  if (rembgStore.busy) return;
  rembgStore.items = rembgStore.items.filter((it) => it.idx !== idx);
}

/** Reassigns idx 0..N-1 in current order so backend's idx-by-position
 *  events line up with our items array. Backend doesn't see the
 *  ever-incrementing nextIdx — it gets a fresh 0..N-1 sequence per run. */
export async function rembgRun() {
  if (rembgStore.busy || rembgStore.items.length === 0) return;
  rembgStore.busy = true;
  rembgStore.errorMsg = "";
  // Reassign idx by current position; reset transient fields.
  rembgStore.items = rembgStore.items.map((it, i) => ({
    ...it,
    idx: i,
    status: "pending",
    pct: 0,
    output: null,
    outputUrl: null,
    error: null,
  }));
  // Reset nextIdx so future additions continue cleanly past the batch.
  nextIdx = rembgStore.items.length;
  try {
    await invoke<unknown>("rembg_run", {
      inputs: rembgStore.items.map((it) => it.input),
      choke: rembgStore.choke,
      model: rembgStore.model,
    });
  } catch (e) {
    rembgStore.errorMsg = String(e);
  } finally {
    rembgStore.busy = false;
  }
}
