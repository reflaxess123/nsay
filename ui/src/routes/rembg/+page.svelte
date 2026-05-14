<script lang="ts">
  import { invoke, convertFileSrc } from "@tauri-apps/api/core";
  import { listen } from "@tauri-apps/api/event";
  import { open as openDialog } from "@tauri-apps/plugin-dialog";
  import { onMount } from "svelte";
  import { scale, fade } from "svelte/transition";
  import { backOut, cubicOut } from "svelte/easing";

  type Status = "pending" | "running" | "done" | "error";
  type Item = {
    idx: number;
    input: string;
    inputUrl: string;
    output: string | null;
    outputUrl: string | null;
    bust: number; // cache-bust counter for output <img> remount
    status: Status;
    pct: number;
    error: string | null;
  };

  let items = $state<Item[]>([]);
  let busy = $state(false);
  let dl = $state<{ id: string; label: string; pct: number } | null>(null);
  let errorMsg = $state("");
  let choke = $state(0.0);
  // Per-item before/after toggle in grid view (id → boolean, true = show input)
  let showInput = $state<Record<number, boolean>>({});

  function addInputs(paths: string[]) {
    if (busy) return;
    const start = items.length;
    const fresh: Item[] = paths.map((p, i) => ({
      idx: start + i,
      input: p,
      inputUrl: convertFileSrc(p),
      output: null,
      outputUrl: null,
      bust: 0,
      status: "pending",
      pct: 0,
      error: null,
    }));
    items = [...items, ...fresh];
  }

  async function pickFiles() {
    const f = await openDialog({
      multiple: true,
      filters: [{ name: "Image", extensions: ["png", "jpg", "jpeg", "webp", "bmp", "tiff"] }],
    });
    if (Array.isArray(f) && f.length) addInputs(f as string[]);
    else if (typeof f === "string") addInputs([f]);
  }

  function clearAll() {
    if (busy) return;
    items = [];
    showInput = {};
    errorMsg = "";
  }

  function removeItem(idx: number) {
    if (busy) return;
    items = items.filter((it) => it.idx !== idx);
  }

  async function run() {
    if (busy || items.length === 0) return;
    busy = true;
    errorMsg = "";
    // Reset transient state but preserve order/idx — backend matches by idx.
    items = items.map((it) => ({ ...it, status: "pending", pct: 0, output: null, outputUrl: null, error: null }));
    try {
      await invoke<unknown>("rembg_run", {
        inputs: items.map((it) => it.input),
        choke,
      });
    } catch (e) {
      errorMsg = String(e);
    } finally {
      busy = false;
    }
  }

  function patchItem(idx: number, patch: Partial<Item>) {
    items = items.map((it) => (it.idx === idx ? { ...it, ...patch } : it));
  }

  onMount(() => {
    const u = [
      // Per-item lifecycle from rembg.rs.
      listen<{ idx: number; input: string; output: string }>("rembg-item-start", (e) => {
        patchItem(e.payload.idx, { status: "running", pct: 0, output: e.payload.output });
      }),
      listen<{ idx: number; pct: number }>("rembg-item-progress", (e) => {
        patchItem(e.payload.idx, { pct: e.payload.pct });
      }),
      listen<{ idx: number; output: string }>("rembg-item-done", (e) => {
        patchItem(e.payload.idx, {
          status: "done",
          pct: 100,
          output: e.payload.output,
          outputUrl: convertFileSrc(e.payload.output),
          bust: Date.now(),
        });
      }),
      listen<{ idx: number; error: string }>("rembg-item-error", (e) => {
        patchItem(e.payload.idx, { status: "error", error: e.payload.error });
      }),

      // First-run model download.
      listen<{ id: string; label: string }>("model-download-start", (e) => {
        dl = { id: e.payload.id, label: e.payload.label, pct: 0 };
      }),
      listen<{ id: string; pct: number }>("model-download-progress", (e) => {
        if (dl && dl.id === e.payload.id) dl.pct = e.payload.pct;
      }),
      listen<{ id: string }>("model-download-done", () => { dl = null; }),
      listen<{ id: string; error: string }>("model-download-error", (e) => {
        errorMsg = `Скачивание модели: ${e.payload.error}`;
        dl = null;
      }),

      // Multi-file drag-drop.
      listen<{ paths: string[] }>("tauri://drag-drop", (e) => {
        if (e.payload.paths?.length) addInputs(e.payload.paths);
      }),
    ];
    return () => { u.forEach((p) => p.then((f) => f())); };
  });

  // Aggregate progress for the bottom strip.
  const total = $derived(items.length);
  const doneCount = $derived(items.filter((it) => it.status === "done").length);
  const errCount = $derived(items.filter((it) => it.status === "error").length);
  const runningItem = $derived(items.find((it) => it.status === "running"));
  const aggregatePct = $derived.by(() => {
    if (total === 0) return 0;
    const sum = items.reduce((s, it) => s + (it.status === "done" ? 100 : it.status === "running" ? it.pct : 0), 0);
    return Math.round(sum / total);
  });
  const statusLabel = $derived.by(() => {
    if (dl) return `Скачиваю ${dl.label} ${dl.pct}%`;
    if (busy && total > 1) return `Обработка ${doneCount + (runningItem ? 1 : 0)}/${total} · ${aggregatePct}%`;
    if (busy && runningItem) return `Обработка ${runningItem.pct}%`;
    if (!busy && total > 0 && doneCount === total) return `Готово ${doneCount}/${total}`;
    if (!busy && errCount > 0) return `Ошибок: ${errCount}`;
    return "";
  });
</script>

<div class="page">
  <section
    class="stage"
    class:has-items={items.length > 0}
    class:running={busy}
  >
    {#if items.length === 0}
      <button class="empty" onclick={pickFiles} aria-label="Выбрать или перетащить файлы" in:fade={{ duration: 200 }}>
        <svg viewBox="0 0 24 24" width="44" height="44" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round">
          <path d="M12 3v12"/><path d="m7 8 5-5 5 5"/><path d="M5 21h14"/>
        </svg>
        <span class="empty-title">Перетащи картинки сюда</span>
        <span class="empty-sub">кликни — выбрать одну или несколько</span>
      </button>
    {:else}
      <div class="grid">
        {#each items as it (it.idx)}
          {@const showIn = showInput[it.idx] ?? false}
          <div
            class="card"
            class:status-pending={it.status === "pending"}
            class:status-running={it.status === "running"}
            class:status-done={it.status === "done"}
            class:status-error={it.status === "error"}
            in:scale={{ start: 0.82, duration: 320, easing: backOut, opacity: 0 }}
            out:scale={{ start: 0.92, duration: 180, easing: cubicOut, opacity: 0 }}
          >
            <div class="thumb">
              <!-- Always render input so it provides the layout/aspect.
                   The output crossfades on top once available. -->
              <img class="thumb-img" src={it.inputUrl} alt="" />
              {#if it.status === "done" && it.outputUrl && !showIn}
                {#key it.bust}
                  <img
                    class="thumb-img out checker"
                    src={it.outputUrl}
                    alt=""
                    in:fade={{ duration: 320, easing: cubicOut }}
                  />
                {/key}
              {/if}

              <!-- Magic shimmer overlay during processing. -->
              {#if it.status === "running"}
                <div class="shimmer" transition:fade={{ duration: 200 }}></div>
                <div class="sparkles" transition:fade={{ duration: 200 }}>
                  <span class="spark s1"></span>
                  <span class="spark s2"></span>
                  <span class="spark s3"></span>
                </div>
              {/if}

              <!-- Per-card progress at the bottom edge of the thumb. -->
              {#if it.status === "running"}
                <div class="card-bar"><div class="card-fill" style:width="{it.pct}%"></div></div>
              {/if}

              <!-- Status badge top-left. -->
              <div class="badge">
                {#if it.status === "pending"}В очереди{/if}
                {#if it.status === "running"}{it.pct}%{/if}
                {#if it.status === "done"}<svg viewBox="0 0 24 24" width="11" height="11" fill="none" stroke="currentColor" stroke-width="2.6" stroke-linecap="round" stroke-linejoin="round"><polyline points="20 6 9 17 4 12"/></svg>{/if}
                {#if it.status === "error"}!{/if}
              </div>

              <!-- Hover tools top-right. -->
              <div class="tools">
                {#if it.status === "done" && it.outputUrl}
                  <button
                    class="tool-btn"
                    onmousedown={() => (showInput[it.idx] = true)}
                    onmouseup={() => (showInput[it.idx] = false)}
                    onmouseleave={() => (showInput[it.idx] = false)}
                    aria-label="Удерживай — показать оригинал"
                    title="Удерживай — оригинал"
                  >
                    <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M1 12s4-8 11-8 11 8 11 8-4 8-11 8-11-8-11-8z"/><circle cx="12" cy="12" r="3"/></svg>
                  </button>
                {/if}
                <button
                  class="tool-btn danger"
                  onclick={() => removeItem(it.idx)}
                  disabled={busy}
                  aria-label="Удалить из очереди"
                >
                  <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M3 6h18"/><path d="M8 6V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2"/><path d="M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6"/></svg>
                </button>
              </div>
            </div>
            <div class="caption">
              <span class="filename" title={it.input}>{it.input.split(/[\\/]/).pop()}</span>
            </div>
          </div>
        {/each}
      </div>
    {/if}
  </section>

  {#if busy || dl}
    <div class="progress-strip" aria-label={statusLabel}>
      <div class="progress-fill" style:width="{dl ? dl.pct : aggregatePct}%"></div>
    </div>
  {/if}

  <footer class="dock">
    <div class="control">
      <label for="choke">Choke</label>
      <input id="choke" type="range" min="0" max="1" step="0.05" bind:value={choke} disabled={busy} />
      <span class="value">{choke.toFixed(2)}</span>
    </div>
    {#if statusLabel}
      <span class="status">{statusLabel}</span>
    {/if}
    <div class="actions">
      {#if errorMsg}
        <span class="error" title={errorMsg}>⚠ {errorMsg.slice(0, 80)}</span>
      {/if}
      {#if items.length > 0}
        <button class="btn-secondary" onclick={clearAll} disabled={busy}>Очистить</button>
      {/if}
      <button class="btn-secondary" onclick={pickFiles} disabled={busy}>
        {items.length > 0 ? "Добавить" : "Выбрать"}
      </button>
      <button class="btn-primary" onclick={run} disabled={items.length === 0 || busy}>
        {busy ? "Работа…" : items.length > 1 ? `Обработать ${items.length}` : "Обработать"}
      </button>
    </div>
  </footer>
</div>

<style>
  .page {
    flex: 1 1 auto;
    min-height: 0;
    display: flex;
    flex-direction: column;
  }

  /* ===== STAGE ===== */
  .stage {
    flex: 1 1 auto;
    min-height: 0;
    position: relative;
    background: var(--surface);
    margin: 0 14px;
    border: 1.5px dashed var(--border);
    border-radius: var(--radius-lg);
    display: flex;
    overflow: hidden;
    transition: border-color 0.2s var(--ease-out);
  }
  .stage.has-items { border-style: solid; }
  .stage.running { border-color: var(--accent); }

  /* Empty state — full-area dropzone. */
  .empty {
    flex: 1;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 8px;
    color: var(--muted);
    background: transparent;
    border: none;
    cursor: pointer;
    font: inherit;
    padding: 0;
  }
  .empty:hover { color: var(--fg); }
  .empty-title { font-size: var(--text-base); font-weight: 600; }
  .empty-sub { font-size: var(--text-xs); }

  /* Grid — auto-fill responsive thumbs. */
  .grid {
    flex: 1;
    min-height: 0;
    overflow: auto;
    padding: var(--space-12);
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(220px, 1fr));
    gap: var(--space-12);
    align-content: start;
  }

  /* ===== CARD ===== */
  .card {
    background: var(--bg-elevated);
    border: 1px solid var(--border);
    border-radius: var(--radius-md);
    overflow: hidden;
    display: flex;
    flex-direction: column;
    transition: border-color 0.2s var(--ease-out), transform 0.2s var(--ease-out);
  }
  .card.status-running { border-color: var(--accent); }
  .card.status-error   { border-color: var(--danger); }
  .card.status-done    { border-color: color-mix(in srgb, var(--ok) 70%, var(--border)); }

  .thumb {
    position: relative;
    aspect-ratio: 1 / 1;
    overflow: hidden;
    background: var(--surface);
  }
  .thumb-img {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    object-fit: contain;
    display: block;
  }
  .thumb-img.out { z-index: 2; }
  /* Checkerboard for transparent PNG output. */
  .checker {
    background-image:
      linear-gradient(45deg, var(--border) 25%, transparent 25%),
      linear-gradient(-45deg, var(--border) 25%, transparent 25%),
      linear-gradient(45deg, transparent 75%, var(--border) 75%),
      linear-gradient(-45deg, transparent 75%, var(--border) 75%);
    background-size: 14px 14px;
    background-position: 0 0, 0 7px, 7px -7px, -7px 0;
  }

  /* ===== MAGIC SHIMMER OVERLAY ===== */
  .shimmer {
    position: absolute;
    inset: 0;
    z-index: 3;
    pointer-events: none;
    background: linear-gradient(
      110deg,
      transparent 30%,
      color-mix(in srgb, var(--accent) 24%, transparent) 50%,
      transparent 70%
    );
    background-size: 220% 100%;
    animation: shimmer 1.4s linear infinite;
    mix-blend-mode: screen;
  }
  @keyframes shimmer {
    0%   { background-position: 200% 0; }
    100% { background-position: -100% 0; }
  }

  /* Sparkle dots — three drifting accents during processing. */
  .sparkles {
    position: absolute;
    inset: 0;
    z-index: 4;
    pointer-events: none;
  }
  .spark {
    position: absolute;
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--accent);
    box-shadow: 0 0 12px 2px var(--accent);
    opacity: 0;
    animation: sparkle 2.6s ease-in-out infinite;
  }
  .spark.s1 { top: 22%; left: 18%; animation-delay: 0s;   }
  .spark.s2 { top: 64%; left: 72%; animation-delay: 0.7s; }
  .spark.s3 { top: 38%; left: 84%; animation-delay: 1.4s; }
  @keyframes sparkle {
    0%, 100% { opacity: 0; transform: scale(0.5); }
    40%      { opacity: 0.95; transform: scale(1); }
    60%      { opacity: 0.95; transform: scale(1); }
  }

  /* Card progress bar at the bottom edge of the thumb. */
  .card-bar {
    position: absolute;
    left: 0; right: 0; bottom: 0;
    height: 3px;
    background: color-mix(in srgb, var(--accent) 18%, transparent);
    z-index: 5;
  }
  .card-fill {
    height: 100%;
    background: var(--accent);
    transition: width 0.18s var(--ease-out);
  }

  /* Badge top-left, tools top-right. */
  .badge {
    position: absolute;
    top: 6px; left: 6px;
    z-index: 6;
    padding: 3px 8px;
    border-radius: var(--radius-pill);
    font: 700 10px / 1 inherit;
    background: color-mix(in srgb, var(--bg-elevated) 80%, transparent);
    backdrop-filter: blur(4px);
    color: var(--muted);
    border: 1px solid var(--border);
    display: inline-flex;
    align-items: center;
    gap: 4px;
  }
  .status-running .badge { color: var(--accent); border-color: var(--accent); }
  .status-done    .badge { color: var(--ok);     border-color: var(--ok); }
  .status-error   .badge { color: var(--danger); border-color: var(--danger); }

  .tools {
    position: absolute;
    top: 6px; right: 6px;
    z-index: 6;
    display: inline-flex;
    gap: 4px;
    opacity: 0;
    transform: translateY(-4px);
    transition: opacity 0.15s var(--ease-out), transform 0.15s var(--ease-out);
  }
  .card:hover .tools { opacity: 1; transform: translateY(0); }
  .tool-btn {
    width: 26px;
    height: 26px;
    appearance: none;
    border: 1px solid var(--border);
    background: color-mix(in srgb, var(--bg-elevated) 88%, transparent);
    backdrop-filter: blur(4px);
    color: var(--fg);
    border-radius: var(--radius-sm);
    cursor: pointer;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    transition: background 0.15s var(--ease-out), color 0.15s var(--ease-out);
  }
  .tool-btn:hover { background: var(--accent); color: var(--accent-fg); border-color: var(--accent); }
  .tool-btn.danger:hover { background: var(--danger); color: #fff; border-color: var(--danger); }

  .caption {
    padding: 6px 8px;
    border-top: 1px solid var(--border);
    background: var(--bg-elevated);
  }
  .filename {
    display: block;
    font: 500 var(--text-xs) / 1.3 inherit;
    color: var(--fg);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  /* ===== PROGRESS STRIP between stage and dock ===== */
  .progress-strip {
    flex: 0 0 auto;
    height: 3px;
    background: color-mix(in srgb, var(--accent) 14%, transparent);
    margin: 6px 14px 0;
    border-radius: var(--radius-pill);
    overflow: hidden;
  }
  .progress-fill {
    height: 100%;
    background: var(--accent);
    transition: width 0.18s var(--ease-out);
    border-radius: inherit;
  }

  /* ===== DOCK ===== */
  .dock {
    flex: 0 0 auto;
    display: flex;
    align-items: center;
    gap: var(--space-12);
    padding: 12px 14px;
  }
  .control {
    display: inline-flex;
    align-items: center;
    gap: var(--space-8);
  }
  .control label { font-weight: 600; color: var(--fg); font-size: var(--text-xs); }
  .control input[type="range"] { width: 120px; accent-color: var(--accent); }
  .control .value {
    font-variant-numeric: tabular-nums;
    color: var(--muted);
    min-width: 28px;
    text-align: right;
    font-size: var(--text-xs);
  }
  .status {
    font: 500 var(--text-xs) / 1 inherit;
    color: var(--muted);
    margin-left: var(--space-12);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    max-width: 320px;
  }
  .actions {
    margin-left: auto;
    display: inline-flex;
    align-items: center;
    gap: var(--space-8);
  }
  .btn-primary, .btn-secondary {
    padding: 8px 16px;
    border-radius: var(--radius-md);
    border: 1px solid transparent;
    cursor: pointer;
    font: 600 var(--text-sm) / 1 inherit;
    transition: background 0.15s var(--ease-out), color 0.15s var(--ease-out);
  }
  .btn-primary { background: var(--accent); color: var(--accent-fg); }
  .btn-primary:hover:not(:disabled) { filter: brightness(1.1); }
  .btn-primary:disabled { opacity: 0.5; cursor: not-allowed; }
  .btn-secondary { background: var(--surface); color: var(--fg); border-color: var(--border); }
  .btn-secondary:hover:not(:disabled) { background: var(--hover); }
  .btn-secondary:disabled { opacity: 0.5; cursor: not-allowed; }

  .error {
    font-size: var(--text-xs);
    color: var(--danger);
    max-width: 320px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
</style>
