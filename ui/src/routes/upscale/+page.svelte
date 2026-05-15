<script lang="ts">
  // Thin UI shell over upscaleStore — see $lib/state/upscale.svelte.ts.
  import { listen } from "@tauri-apps/api/event";
  import { open as openDialog } from "@tauri-apps/plugin-dialog";
  import { onMount } from "svelte";
  import { scale, fade } from "svelte/transition";
  import { backOut, cubicOut } from "svelte/easing";
  import {
    upscaleStore,
    upscaleInit,
    upscaleAddInputs,
    upscaleClear,
    upscaleRemove,
    upscaleRun,
  } from "$lib/state/upscale.svelte.ts";
  import SidePanel from "$lib/components/SidePanel.svelte";

  // Pretty label for the scale toggle. ×1.5 instead of ×1.5 to keep widths
  // tight; integers stay bare.
  function scaleLabel(s: number): string {
    return Number.isInteger(s) ? `×${s}` : `×${s.toFixed(1)}`;
  }
  // Active model entry — used to gate scale buttons against the model's
  // native ratio (e.g. an x2 model can't honor ×4 cleanly).
  const activeModel = $derived(
    upscaleStore.models.find((m) => m.id === upscaleStore.model),
  );
  const maxScale = $derived(activeModel?.output_scale ?? 4);

  async function pickFiles() {
    const f = await openDialog({
      multiple: true,
      filters: [{ name: "Image", extensions: ["png", "jpg", "jpeg", "webp", "bmp", "tiff"] }],
    });
    if (Array.isArray(f) && f.length) upscaleAddInputs(f as string[]);
    else if (typeof f === "string") upscaleAddInputs([f]);
  }

  onMount(() => {
    upscaleInit();
    const u = listen<{ paths: string[] }>("tauri://drag-drop", (e) => {
      if (e.payload.paths?.length) upscaleAddInputs(e.payload.paths);
    });
    return () => { u.then((f) => f()); };
  });

  const total = $derived(upscaleStore.items.length);
  const doneCount = $derived(upscaleStore.items.filter((it) => it.status === "done").length);
  const errCount = $derived(upscaleStore.items.filter((it) => it.status === "error").length);
  const runningItem = $derived(upscaleStore.items.find((it) => it.status === "running"));
  const aggregatePct = $derived.by(() => {
    if (total === 0) return 0;
    const sum = upscaleStore.items.reduce(
      (s, it) => s + (it.status === "done" ? 100 : it.status === "running" ? it.pct : 0),
      0,
    );
    return Math.round(sum / total);
  });
  const statusLabel = $derived.by(() => {
    if (upscaleStore.dl) return `Скачиваю ${upscaleStore.dl.label} ${upscaleStore.dl.pct}%`;
    if (upscaleStore.busy && total > 1) return `Обработка ${doneCount + (runningItem ? 1 : 0)}/${total} · ${aggregatePct}%`;
    if (upscaleStore.busy && runningItem) return `Обработка ${runningItem.pct}%`;
    if (!upscaleStore.busy && total > 0 && doneCount === total) return `Готово ${doneCount}/${total} · ×${upscaleStore.scale}`;
    if (!upscaleStore.busy && errCount > 0) return `Ошибок: ${errCount}`;
    return "";
  });
</script>

<div class="page">
  <div class="body">
    <div class="work">
  <section class="stage" class:has-items={upscaleStore.items.length > 0} class:running={upscaleStore.busy}>
    {#if upscaleStore.items.length === 0}
      <button class="empty" onclick={pickFiles} aria-label="Выбрать или перетащить файлы" in:fade={{ duration: 200 }}>
        <svg viewBox="0 0 24 24" width="44" height="44" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round">
          <path d="M12 3v13"/><path d="m7 12 5 5 5-5"/><path d="M5 21h14"/>
        </svg>
        <span class="empty-title">Перетащи картинки сюда</span>
        <span class="empty-sub">Real-ESRGAN апскейл ×{upscaleStore.scale}</span>
      </button>
    {:else}
      <div class="grid">
        {#each upscaleStore.items as it (it.idx)}
          {@const showIn = upscaleStore.showInput[it.idx] ?? false}
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
              <img class="thumb-img" src={it.inputUrl} alt="" />
              {#if it.status === "done" && it.outputUrl && !showIn}
                {#key it.bust}
                  <img class="thumb-img out" src={it.outputUrl} alt="" in:fade={{ duration: 320, easing: cubicOut }} />
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
                {#if it.status === "done"}<svg viewBox="0 0 24 24" width="11" height="11" fill="none" stroke="currentColor" stroke-width="2.6" stroke-linecap="round" stroke-linejoin="round"><polyline points="20 6 9 17 4 12"/></svg> ×{upscaleStore.scale}{/if}
                {#if it.status === "error"}!{/if}
              </div>

              <div class="tools">
                {#if it.status === "done" && it.outputUrl}
                  <button
                    class="tool-btn"
                    onmousedown={() => (upscaleStore.showInput[it.idx] = true)}
                    onmouseup={() => (upscaleStore.showInput[it.idx] = false)}
                    onmouseleave={() => (upscaleStore.showInput[it.idx] = false)}
                    aria-label="Удерживай — оригинал" title="Удерживай — оригинал"
                  >
                    <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M1 12s4-8 11-8 11 8 11 8-4 8-11 8-11-8-11-8z"/><circle cx="12" cy="12" r="3"/></svg>
                  </button>
                {/if}
                <button class="tool-btn danger" onclick={() => upscaleRemove(it.idx)} disabled={upscaleStore.busy} aria-label="Удалить из очереди">
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

  {#if upscaleStore.busy || upscaleStore.dl}
    <div class="progress-strip" aria-label={statusLabel}>
      <div class="progress-fill" style:width="{upscaleStore.dl ? upscaleStore.dl.pct : aggregatePct}%"></div>
    </div>
  {/if}

  <footer class="dock">
    <div class="scale-toggle" role="group" aria-label="Множитель апскейла">
      {#each [1.5, 2, 3, 4] as s (s)}
        <button
          type="button"
          class="scale-btn"
          class:active={upscaleStore.scale === s}
          disabled={upscaleStore.busy || s > maxScale}
          title={s > maxScale ? `Модель ×${maxScale} не вытянет ×${s} без потери качества` : ""}
          onclick={() => (upscaleStore.scale = s as 1.5 | 2 | 3 | 4)}
        >{scaleLabel(s)}</button>
      {/each}
    </div>
    {#if statusLabel}
      <span class="status">{statusLabel}</span>
    {/if}
    <div class="actions">
      {#if upscaleStore.errorMsg}
        <span class="error" title={upscaleStore.errorMsg}>⚠ {upscaleStore.errorMsg.slice(0, 80)}</span>
      {/if}
      {#if upscaleStore.items.length > 0}
        <button class="btn-secondary" onclick={upscaleClear} disabled={upscaleStore.busy}>Очистить</button>
      {/if}
      <button class="btn-secondary" onclick={pickFiles} disabled={upscaleStore.busy}>
        {upscaleStore.items.length > 0 ? "Добавить" : "Выбрать"}
      </button>
      <button class="btn-primary" onclick={upscaleRun} disabled={upscaleStore.items.length === 0 || upscaleStore.busy}>
        {upscaleStore.busy ? "Работа…" : upscaleStore.items.length > 1 ? `Апскейл ${upscaleStore.items.length}` : "Апскейл"}
      </button>
    </div>
  </footer>
    </div>
    <SidePanel family="upscale" bind:model={upscaleStore.model} disabled={upscaleStore.busy} />
  </div>
</div>

<style>
  /* Layout / cards / animations match rembg/+page.svelte 1:1.
     Keep them in sync if you tweak one. */
  .page { flex: 1 1 auto; min-height: 0; display: flex; flex-direction: column; }
  .body { flex: 1 1 auto; min-height: 0; display: flex; }
  .work { flex: 1 1 auto; min-width: 0; min-height: 0; display: flex; flex-direction: column; }
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
  /* `cover` so input + output share the same crop — otherwise upscaled
     output can overlap the centered input behind it visibly. */
  .thumb-img { position: absolute; inset: 0; width: 100%; height: 100%; object-fit: cover; display: block; }
  .thumb-img.out { z-index: 2; }

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
  .scale-toggle {
    display: inline-flex;
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--radius-pill);
    padding: 2px;
    gap: 0;
  }
  .scale-btn {
    appearance: none;
    border: none;
    background: transparent;
    color: var(--muted);
    font: 700 var(--text-xs) / 1 inherit;
    padding: 6px 12px;
    border-radius: var(--radius-pill);
    cursor: pointer;
    transition: background 0.15s var(--ease-out), color 0.15s var(--ease-out);
  }
  .scale-btn:hover:not(:disabled):not(.active) { color: var(--fg); }
  .scale-btn.active { background: var(--accent); color: var(--accent-fg); }
  .scale-btn:disabled { opacity: 0.5; cursor: not-allowed; }
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
