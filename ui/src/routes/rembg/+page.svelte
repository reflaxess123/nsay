<script lang="ts">
  // Thin UI shell over rembgStore — all state and listeners live in
  // $lib/state/rembg.svelte.ts so switching routes (Rembg → Settings →
  // Rembg) doesn't drop the queue or stop reacting to backend events.
  import { listen } from "@tauri-apps/api/event";
  import { open as openDialog } from "@tauri-apps/plugin-dialog";
  import { onMount } from "svelte";
  import { scale, fade } from "svelte/transition";
  import { backOut, cubicOut } from "svelte/easing";
  import {
    rembgStore,
    rembgInit,
    rembgAddInputs,
    rembgClear,
    rembgRemove,
    rembgRun,
  } from "$lib/state/rembg.svelte.ts";
  import SidePanel from "$lib/components/SidePanel.svelte";

  async function pickFiles() {
    const f = await openDialog({
      multiple: true,
      filters: [{ name: "Image", extensions: ["png", "jpg", "jpeg", "webp", "bmp", "tiff"] }],
    });
    if (Array.isArray(f) && f.length) rembgAddInputs(f as string[]);
    else if (typeof f === "string") rembgAddInputs([f]);
  }

  onMount(() => {
    rembgInit();
    // Drag-drop is a window event — only relevant when this route is on
    // screen, so we register on mount and unregister on unmount. Items
    // live in the store so they survive unmount.
    const u = listen<{ paths: string[] }>("tauri://drag-drop", (e) => {
      if (e.payload.paths?.length) rembgAddInputs(e.payload.paths);
    });
    return () => { u.then((f) => f()); };
  });

  // Aggregate progress for the bottom strip.
  const total = $derived(rembgStore.items.length);
  const doneCount = $derived(rembgStore.items.filter((it) => it.status === "done").length);
  const errCount = $derived(rembgStore.items.filter((it) => it.status === "error").length);
  const runningItem = $derived(rembgStore.items.find((it) => it.status === "running"));
  const aggregatePct = $derived.by(() => {
    if (total === 0) return 0;
    const sum = rembgStore.items.reduce(
      (s, it) => s + (it.status === "done" ? 100 : it.status === "running" ? it.pct : 0),
      0,
    );
    return Math.round(sum / total);
  });
  const statusLabel = $derived.by(() => {
    if (rembgStore.dl) return `Скачиваю ${rembgStore.dl.label} ${rembgStore.dl.pct}%`;
    if (rembgStore.busy && total > 1) return `Обработка ${doneCount + (runningItem ? 1 : 0)}/${total} · ${aggregatePct}%`;
    if (rembgStore.busy && runningItem) return `Обработка ${runningItem.pct}%`;
    if (!rembgStore.busy && total > 0 && doneCount === total) return `Готово ${doneCount}/${total}`;
    if (!rembgStore.busy && errCount > 0) return `Ошибок: ${errCount}`;
    return "";
  });
</script>

<div class="page">
  <div class="body">
    <div class="work">
  <section class="stage" class:has-items={rembgStore.items.length > 0} class:running={rembgStore.busy}>
    {#if rembgStore.items.length === 0}
      <button class="empty" onclick={pickFiles} aria-label="Выбрать или перетащить файлы" in:fade={{ duration: 200 }}>
        <svg viewBox="0 0 24 24" width="44" height="44" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round">
          <path d="M12 3v13"/><path d="m7 12 5 5 5-5"/><path d="M5 21h14"/>
        </svg>
        <span class="empty-title">Перетащи картинки сюда</span>
        <span class="empty-sub">кликни — выбрать одну или несколько</span>
      </button>
    {:else}
      <div class="grid">
        {#each rembgStore.items as it (it.idx)}
          {@const showIn = rembgStore.showInput[it.idx] ?? false}
          {@const showOut = it.status === "done" && it.outputUrl && !showIn}
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
              <img class="thumb-img" class:hidden={showOut} src={it.inputUrl} alt="" />
              {#if showOut}
                {#key it.bust}
                  <img class="thumb-img out checker" src={it.outputUrl} alt="" in:fade={{ duration: 320, easing: cubicOut }} />
                {/key}
              {/if}

              {#if it.status === "running"}
                <div class="shimmer" transition:fade={{ duration: 200 }}></div>
                <div class="sparkles" transition:fade={{ duration: 200 }}>
                  <span class="spark s1"></span>
                  <span class="spark s2"></span>
                  <span class="spark s3"></span>
                </div>
                <div class="card-bar"><div class="card-fill" style:width="{it.pct}%"></div></div>
              {/if}

              <div class="badge">
                {#if it.status === "pending"}В очереди{/if}
                {#if it.status === "running"}{it.pct}%{/if}
                {#if it.status === "done"}<svg viewBox="0 0 24 24" width="11" height="11" fill="none" stroke="currentColor" stroke-width="2.6" stroke-linecap="round" stroke-linejoin="round"><polyline points="20 6 9 17 4 12"/></svg>{/if}
                {#if it.status === "error"}!{/if}
              </div>

              <div class="tools">
                {#if it.status === "done" && it.outputUrl}
                  <button
                    class="tool-btn"
                    onmousedown={() => (rembgStore.showInput[it.idx] = true)}
                    onmouseup={() => (rembgStore.showInput[it.idx] = false)}
                    onmouseleave={() => (rembgStore.showInput[it.idx] = false)}
                    aria-label="Удерживай — оригинал" title="Удерживай — оригинал"
                  >
                    <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M1 12s4-8 11-8 11 8 11 8-4 8-11 8-11-8-11-8z"/><circle cx="12" cy="12" r="3"/></svg>
                  </button>
                {/if}
                <button class="tool-btn danger" onclick={() => rembgRemove(it.idx)} disabled={rembgStore.busy} aria-label="Удалить из очереди">
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

  {#if rembgStore.busy || rembgStore.dl}
    <div class="progress-strip" aria-label={statusLabel}>
      <div class="progress-fill" style:width="{rembgStore.dl ? rembgStore.dl.pct : aggregatePct}%"></div>
    </div>
  {/if}

  <footer class="dock">
    <div class="control">
      <label for="choke">Choke</label>
      <input
        id="choke"
        type="range"
        min="0"
        max="1"
        step="0.05"
        bind:value={rembgStore.choke}
        disabled={rembgStore.busy}
        style:--pct="{(rembgStore.choke * 100).toFixed(0)}"
      />
      <span class="value">{rembgStore.choke.toFixed(2)}</span>
    </div>
    {#if statusLabel}
      <span class="status">{statusLabel}</span>
    {/if}
    <div class="actions">
      {#if rembgStore.errorMsg}
        <span class="error" title={rembgStore.errorMsg}>⚠ {rembgStore.errorMsg.slice(0, 80)}</span>
      {/if}
      {#if rembgStore.items.length > 0}
        <button class="btn-secondary" onclick={rembgClear} disabled={rembgStore.busy}>Очистить</button>
      {/if}
      <button class="btn-secondary" onclick={pickFiles} disabled={rembgStore.busy}>
        {rembgStore.items.length > 0 ? "Добавить" : "Выбрать"}
      </button>
      <button class="btn-primary" onclick={rembgRun} disabled={rembgStore.items.length === 0 || rembgStore.busy}>
        {rembgStore.busy ? "Работа…" : rembgStore.items.length > 1 ? `Обработать ${rembgStore.items.length}` : "Обработать"}
      </button>
    </div>
  </footer>
    </div>
    <SidePanel family="rembg" bind:model={rembgStore.model} disabled={rembgStore.busy} />
  </div>
</div>

<style>
  /* Layout / cards / animations match upscale/+page.svelte 1:1.
     Keep them in sync if you tweak one. */
  .page { flex: 1 1 auto; min-height: 0; display: flex; flex-direction: column; }
  /* row-flex: working area on the left fills the rest, SidePanel pinned right. */
  .body { flex: 1 1 auto; min-height: 0; display: flex; }
  .work { flex: 1 1 auto; min-width: 0; min-height: 0; display: flex; flex-direction: column; }
  /* Flat stage. When empty, the drop-zone box centres inside it instead
     of spanning the whole area. When populated, it hosts the cards grid. */
  .stage {
    flex: 1 1 auto;
    min-height: 0;
    position: relative;
    display: flex;
    align-items: center;
    justify-content: center;
    overflow: hidden;
  }
  .stage.has-items {
    align-items: stretch;
    justify-content: stretch;
  }

  /* Compact drop-target — sits in the centre of an empty stage rather
     than swallowing it whole. Border stays dashed to keep the "drop here"
     affordance without making the whole window look unfinished. */
  .empty {
    width: 320px;
    max-width: 80%;
    padding: 28px 24px;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 10px;
    color: var(--muted);
    background: var(--surface);
    border: 1.5px dashed var(--border);
    border-radius: var(--radius-lg);
    cursor: pointer;
    font: inherit;
    transition: border-color 0.2s var(--ease-out), color 0.2s var(--ease-out), background 0.2s var(--ease-out);
  }
  .empty:hover {
    color: var(--fg);
    border-color: var(--accent);
    background: var(--bg-elevated);
  }
  .empty-title { font-size: var(--text-base); font-weight: 600; }
  .empty-sub { font-size: var(--text-xs); }

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

  .card {
    background: var(--bg-elevated);
    border: 1px solid var(--border);
    border-radius: var(--radius-md);
    overflow: hidden;
    display: flex;
    flex-direction: column;
    transition: border-color 0.2s var(--ease-out);
  }
  .card.status-running { border-color: var(--accent); }
  .card.status-error   { border-color: var(--danger); }
  .card.status-done    { border-color: color-mix(in srgb, var(--ok) 70%, var(--border)); }

  .thumb { position: relative; aspect-ratio: 1 / 1; overflow: hidden; background: var(--surface); }
  /* `cover` so the image fills the square thumb (cropping sides as needed)
     instead of letterboxing. Keeps input + output exactly the same crop
     so the output doesn't "float" over an oversized input below it. */
  .thumb-img { position: absolute; inset: 0; width: 100%; height: 100%; object-fit: cover; display: block; }
  .thumb-img.out { z-index: 2; }
  /* Hide the input layer once output is on top — otherwise the original
     bleeds through the transparent pixels of the rembg result instead of
     the checker pattern. */
  .thumb-img.hidden { visibility: hidden; }
  .checker {
    background-image:
      linear-gradient(45deg, var(--border) 25%, transparent 25%),
      linear-gradient(-45deg, var(--border) 25%, transparent 25%),
      linear-gradient(45deg, transparent 75%, var(--border) 75%),
      linear-gradient(-45deg, transparent 75%, var(--border) 75%);
    background-size: 14px 14px;
    background-position: 0 0, 0 7px, 7px -7px, -7px 0;
  }

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
  .sparkles { position: absolute; inset: 0; z-index: 4; pointer-events: none; }
  .spark {
    position: absolute;
    width: 6px; height: 6px;
    border-radius: 50%;
    background: var(--accent);
    box-shadow: 0 0 12px 2px var(--accent);
    opacity: 0;
    animation: sparkle 2.6s ease-in-out infinite;
  }
  .spark.s1 { top: 22%; left: 18%; }
  .spark.s2 { top: 64%; left: 72%; animation-delay: 0.7s; }
  .spark.s3 { top: 38%; left: 84%; animation-delay: 1.4s; }
  @keyframes sparkle {
    0%, 100% { opacity: 0; transform: scale(0.5); }
    40%, 60% { opacity: 0.95; transform: scale(1); }
  }
  .card-bar {
    position: absolute;
    left: 0; right: 0; bottom: 0;
    height: 3px;
    background: color-mix(in srgb, var(--accent) 18%, transparent);
    z-index: 5;
  }
  .card-fill { height: 100%; background: var(--accent); transition: width 0.18s var(--ease-out); }

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
    display: inline-flex; align-items: center; gap: 4px;
  }
  .status-running .badge { color: var(--accent); border-color: var(--accent); }
  .status-done    .badge { color: var(--ok);     border-color: var(--ok); }
  .status-error   .badge { color: var(--danger); border-color: var(--danger); }

  .tools {
    position: absolute;
    top: 6px; right: 6px;
    z-index: 6;
    display: inline-flex; gap: 4px;
    opacity: 0; transform: translateY(-4px);
    transition: opacity 0.15s var(--ease-out), transform 0.15s var(--ease-out);
  }
  .card:hover .tools { opacity: 1; transform: translateY(0); }
  .tool-btn {
    width: 26px; height: 26px;
    appearance: none;
    border: 1px solid var(--border);
    background: color-mix(in srgb, var(--bg-elevated) 88%, transparent);
    backdrop-filter: blur(4px);
    color: var(--fg);
    border-radius: var(--radius-sm);
    cursor: pointer;
    display: inline-flex; align-items: center; justify-content: center;
    transition: background 0.15s var(--ease-out), color 0.15s var(--ease-out);
  }
  .tool-btn:hover { background: var(--accent); color: var(--accent-fg); border-color: var(--accent); }
  .tool-btn.danger:hover { background: var(--danger); color: #fff; border-color: var(--danger); }

  .caption { padding: 6px 8px; border-top: 1px solid var(--border); background: var(--bg-elevated); }
  .filename {
    display: block;
    font: 500 var(--text-xs) / 1.3 inherit;
    color: var(--fg);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .progress-strip {
    flex: 0 0 auto;
    height: 3px;
    background: color-mix(in srgb, var(--accent) 14%, transparent);
    margin: 6px 14px 0;
    border-radius: var(--radius-pill);
    overflow: hidden;
  }
  .progress-fill {
    height: 100%; background: var(--accent);
    transition: width 0.18s var(--ease-out);
    border-radius: inherit;
  }

  .dock {
    flex: 0 0 auto;
    display: flex; align-items: center;
    gap: var(--space-12);
    padding: 12px 14px;
  }
  .control { display: inline-flex; align-items: center; gap: var(--space-8); }
  .control label { font-weight: 600; color: var(--fg); font-size: var(--text-xs); }

  /* Custom range slider — flat track with a filled portion up to the thumb,
     pill-shaped, thumb is an accent-bordered dot that nudges on hover/active.
     The fill is driven by a `--pct` CSS variable set inline from Svelte
     (0..100). All four pseudo-element variants (webkit/moz, track/thumb)
     have to be styled separately because browsers won't share rules across
     prefixes. */
  .control input[type="range"] {
    appearance: none;
    -webkit-appearance: none;
    width: 140px;
    height: 22px;
    background: transparent;
    cursor: pointer;
    margin: 0;
    --pct: 0;
  }
  .control input[type="range"]:disabled { opacity: 0.5; cursor: not-allowed; }

  .control input[type="range"]::-webkit-slider-runnable-track {
    height: 5px;
    border-radius: var(--radius-pill);
    background:
      linear-gradient(
        to right,
        var(--accent) 0%,
        var(--accent) calc(var(--pct) * 1%),
        color-mix(in srgb, var(--accent) 14%, transparent) calc(var(--pct) * 1%),
        color-mix(in srgb, var(--accent) 14%, transparent) 100%
      );
  }
  .control input[type="range"]::-webkit-slider-thumb {
    appearance: none;
    -webkit-appearance: none;
    width: 16px;
    height: 16px;
    border-radius: 50%;
    background: var(--bg-elevated);
    border: 2.5px solid var(--accent);
    margin-top: -5.5px;
    box-shadow: 0 2px 6px rgba(0, 0, 0, 0.18);
    transition: transform 0.15s var(--ease-out), box-shadow 0.15s var(--ease-out);
  }
  .control input[type="range"]:hover::-webkit-slider-thumb {
    transform: scale(1.12);
    box-shadow: 0 3px 10px color-mix(in srgb, var(--accent) 35%, transparent);
  }
  .control input[type="range"]:active::-webkit-slider-thumb {
    transform: scale(1.2);
  }

  .control input[type="range"]::-moz-range-track {
    height: 5px;
    border-radius: var(--radius-pill);
    background: color-mix(in srgb, var(--accent) 14%, transparent);
  }
  .control input[type="range"]::-moz-range-progress {
    height: 5px;
    border-radius: var(--radius-pill);
    background: var(--accent);
  }
  .control input[type="range"]::-moz-range-thumb {
    width: 16px;
    height: 16px;
    border-radius: 50%;
    background: var(--bg-elevated);
    border: 2.5px solid var(--accent);
    box-shadow: 0 2px 6px rgba(0, 0, 0, 0.18);
    cursor: pointer;
    transition: transform 0.15s var(--ease-out);
  }
  .control input[type="range"]:hover::-moz-range-thumb { transform: scale(1.12); }

  .control .value {
    font-variant-numeric: tabular-nums;
    color: var(--muted);
    min-width: 28px;
    text-align: right;
    font-size: var(--text-xs);
    font-weight: 600;
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
  .actions { margin-left: auto; display: inline-flex; align-items: center; gap: var(--space-8); }
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
