<script lang="ts">
  import { listen } from "@tauri-apps/api/event";
  import { open as openDialog } from "@tauri-apps/plugin-dialog";
  import { onMount } from "svelte";
  import SidePanel from "$lib/components/SidePanel.svelte";
  import {
    vidUpscaleStore,
    vidUpscaleInit,
    vidUpscaleSetInput,
    vidUpscaleClear,
    vidUpscaleRun,
  } from "$lib/state/vidUpscale.svelte.ts";

  async function pickFile() {
    const f = await openDialog({
      multiple: false,
      filters: [{ name: "Video", extensions: ["mp4", "mov", "mkv", "webm", "avi"] }],
    });
    if (typeof f === "string") vidUpscaleSetInput(f);
  }

  onMount(() => {
    vidUpscaleInit();
    const u = listen<{ paths: string[] }>("tauri://drag-drop", (e) => {
      if (e.payload.paths?.length) vidUpscaleSetInput(e.payload.paths[0]);
    });
    return () => { u.then((f) => f()); };
  });

  function scaleLabel(s: number): string {
    return Number.isInteger(s) ? `×${s}` : `×${s.toFixed(1)}`;
  }
  // Match upscale page: gate scale buttons against the current model's
  // native ratio (1.5/2 work on x2 model, x3/x4 don't).
  // We don't have direct access to model output_scale here without invoke;
  // SidePanel handles the same call. For now allow all four.

  const fname = $derived.by(() => {
    const p = vidUpscaleStore.input;
    if (!p) return "";
    return p.split(/[\\/]/).pop() ?? p;
  });
  const statusLabel = $derived.by(() => {
    const s = vidUpscaleStore.status;
    if (s === "running") {
      const p = vidUpscaleStore.probe;
      const f = vidUpscaleStore.frame;
      const t = p?.total_frames ?? 0;
      return `${vidUpscaleStore.pct}% · кадр ${f}${t ? `/${t}` : ""}`;
    }
    if (s === "done") return `Готово ×${scaleLabel(vidUpscaleStore.scale).slice(1)} · ${vidUpscaleStore.probe?.encoder ?? ""}`;
    if (s === "error") return "Ошибка";
    return "";
  });
</script>

<div class="page">
  <div class="body">
    <div class="work">
      <section class="stage">
        {#if !vidUpscaleStore.input}
          <button class="empty" onclick={pickFile}>
            <svg viewBox="0 0 24 24" width="44" height="44" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round">
              <path d="M12 3v13"/><path d="m7 12 5 5 5-5"/><path d="M5 21h14"/>
            </svg>
            <span class="empty-title">Перетащи видео сюда</span>
            <span class="empty-sub">mp4 / mov / mkv / webm</span>
          </button>
        {:else}
          <div class="players">
            <div class="player">
              <div class="player-label">Источник</div>
              <video src={vidUpscaleStore.inputUrl} controls muted playsinline></video>
              <div class="player-meta">
                {fname}
                {#if vidUpscaleStore.probe}
                  · {vidUpscaleStore.probe.src_w}×{vidUpscaleStore.probe.src_h}
                {/if}
              </div>
            </div>
            {#if vidUpscaleStore.outputUrl}
              <div class="player">
                <div class="player-label out">Результат</div>
                {#key vidUpscaleStore.bust}
                  <video src={vidUpscaleStore.outputUrl} controls muted playsinline></video>
                {/key}
                <div class="player-meta">
                  {#if vidUpscaleStore.probe}
                    {vidUpscaleStore.probe.out_w}×{vidUpscaleStore.probe.out_h} · {vidUpscaleStore.probe.encoder}
                  {/if}
                </div>
              </div>
            {/if}
          </div>
        {/if}
      </section>

      {#if vidUpscaleStore.status === "running"}
        <div class="progress-strip">
          <div class="progress-fill" style:width="{vidUpscaleStore.pct}%"></div>
        </div>
      {/if}

      <footer class="dock">
        <div class="scale-toggle" role="group" aria-label="Множитель">
          {#each [1.5, 2, 3, 4] as s (s)}
            <button
              type="button"
              class="scale-btn"
              class:active={vidUpscaleStore.scale === s}
              disabled={vidUpscaleStore.status === "running"}
              onclick={() => (vidUpscaleStore.scale = s as 1.5 | 2 | 3 | 4)}
            >{scaleLabel(s)}</button>
          {/each}
        </div>
        {#if statusLabel}
          <span class="status">{statusLabel}</span>
        {/if}
        <div class="actions">
          {#if vidUpscaleStore.errorMsg}
            <span class="error" title={vidUpscaleStore.errorMsg}>⚠ {vidUpscaleStore.errorMsg.slice(0, 80)}</span>
          {/if}
          {#if vidUpscaleStore.input}
            <button class="btn-secondary" onclick={vidUpscaleClear} disabled={vidUpscaleStore.status === "running"}>Сбросить</button>
          {/if}
          <button class="btn-secondary" onclick={pickFile} disabled={vidUpscaleStore.status === "running"}>
            {vidUpscaleStore.input ? "Сменить" : "Выбрать"}
          </button>
          <button class="btn-primary" onclick={vidUpscaleRun} disabled={!vidUpscaleStore.input || vidUpscaleStore.status === "running"}>
            {vidUpscaleStore.status === "running" ? "Работа…" : "Апскейл"}
          </button>
        </div>
      </footer>
    </div>
    <SidePanel family="upscale" bind:model={vidUpscaleStore.model} disabled={vidUpscaleStore.status === "running"} />
  </div>
</div>

<style>
  .page { flex: 1 1 auto; min-height: 0; display: flex; flex-direction: column; }
  .body { flex: 1 1 auto; min-height: 0; display: flex; }
  .work { flex: 1 1 auto; min-width: 0; min-height: 0; display: flex; flex-direction: column; }
  .stage { flex: 1 1 auto; min-height: 0; position: relative; display: flex; align-items: center; justify-content: center; overflow: hidden; padding: var(--space-12); }

  .empty {
    width: 320px; max-width: 80%; padding: 28px 24px;
    display: flex; flex-direction: column; align-items: center; justify-content: center;
    gap: 10px; color: var(--muted); background: var(--surface);
    border: 1.5px dashed var(--border); border-radius: var(--radius-lg);
    cursor: pointer; font: inherit;
    transition: border-color 0.2s var(--ease-out), color 0.2s var(--ease-out), background 0.2s var(--ease-out);
  }
  .empty:hover { color: var(--fg); border-color: var(--accent); background: var(--bg-elevated); }
  .empty-title { font-size: var(--text-base); font-weight: 600; }
  .empty-sub { font-size: var(--text-xs); }

  /* Two-column when output exists; single column otherwise. */
  .players {
    flex: 1 1 auto;
    min-height: 0;
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(360px, 1fr));
    gap: var(--space-12);
    width: 100%;
  }
  .player {
    display: flex;
    flex-direction: column;
    gap: 6px;
    min-height: 0;
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--radius-md);
    padding: 8px;
  }
  .player video {
    width: 100%;
    flex: 1 1 auto;
    min-height: 0;
    border-radius: var(--radius-sm);
    background: #000;
    object-fit: contain;
  }
  .player-label {
    font: 700 9px / 1 inherit;
    text-transform: uppercase;
    letter-spacing: 0.6px;
    color: var(--muted);
  }
  .player-label.out { color: var(--ok); }
  .player-meta {
    font: 500 var(--text-xs) / 1.3 inherit;
    color: var(--muted);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .progress-strip {
    flex: 0 0 auto; height: 3px;
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
  }
  .scale-btn {
    appearance: none; border: none; background: transparent;
    color: var(--muted); font: 700 var(--text-xs) / 1 inherit;
    padding: 6px 12px; border-radius: var(--radius-pill);
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
    font-size: var(--text-xs); color: var(--danger);
    max-width: 320px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap;
  }
</style>
