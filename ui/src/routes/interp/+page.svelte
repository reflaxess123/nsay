<script lang="ts">
  import { listen } from "@tauri-apps/api/event";
  import { open as openDialog } from "@tauri-apps/plugin-dialog";
  import { onMount } from "svelte";
  import SidePanel from "$lib/components/SidePanel.svelte";
  import {
    vidSlowStore,
    vidSlowInit,
    vidSlowSetInput,
    vidSlowClear,
    vidSlowRun,
  } from "$lib/state/vidSlow.svelte.ts";

  async function pickFile() {
    const f = await openDialog({
      multiple: false,
      filters: [{ name: "Video", extensions: ["mp4", "mov", "mkv", "webm", "avi"] }],
    });
    if (typeof f === "string") vidSlowSetInput(f);
  }

  onMount(() => {
    vidSlowInit();
    const u = listen<{ paths: string[] }>("tauri://drag-drop", (e) => {
      if (e.payload.paths?.length) vidSlowSetInput(e.payload.paths[0]);
    });
    return () => { u.then((f) => f()); };
  });

  const fname = $derived.by(() => {
    const p = vidSlowStore.input;
    return p ? (p.split(/[\\/]/).pop() ?? p) : "";
  });
  const fpsLabel = $derived.by(() => {
    const p = vidSlowStore.probe;
    if (!p) return "";
    return (p.fps_num / p.fps_den).toFixed(2);
  });
  const statusLabel = $derived.by(() => {
    const s = vidSlowStore.status;
    if (s === "running") {
      const t = vidSlowStore.probe?.total_frames ?? 0;
      return `${vidSlowStore.pct}% · кадр ${vidSlowStore.frame}${t ? `/${t}` : ""}`;
    }
    if (s === "done") return `Готово ×${vidSlowStore.factor} · ${vidSlowStore.probe?.encoder ?? ""}`;
    if (s === "error") return "Ошибка";
    return "";
  });
</script>

<div class="page">
  <div class="body">
    <div class="work">
      <section class="stage">
        {#if !vidSlowStore.input}
          <button class="empty" onclick={pickFile}>
            <svg viewBox="0 0 24 24" width="44" height="44" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round">
              <path d="M12 3v13"/><path d="m7 12 5 5 5-5"/><path d="M5 21h14"/>
            </svg>
            <span class="empty-title">Перетащи видео сюда</span>
            <span class="empty-sub">RIFE интерполяция · 30→60→120 fps</span>
          </button>
        {:else}
          <div class="players">
            <div class="player">
              <div class="player-label">Источник</div>
              <video src={vidSlowStore.inputUrl} controls muted playsinline></video>
              <div class="player-meta">
                {fname}
                {#if vidSlowStore.probe}
                  · {vidSlowStore.probe.src_w}×{vidSlowStore.probe.src_h} · {fpsLabel} fps
                {/if}
              </div>
            </div>
            {#if vidSlowStore.outputUrl}
              <div class="player">
                <div class="player-label out">Slow-mo ×{vidSlowStore.factor}</div>
                {#key vidSlowStore.bust}
                  <video src={vidSlowStore.outputUrl} controls muted playsinline></video>
                {/key}
                <div class="player-meta">
                  {#if vidSlowStore.probe}
                    {(vidSlowStore.probe.fps_num / vidSlowStore.probe.fps_den).toFixed(2)} fps · {vidSlowStore.probe.encoder}
                  {/if}
                </div>
              </div>
            {/if}
          </div>
        {/if}
      </section>

      {#if vidSlowStore.status === "running"}
        <div class="progress-strip">
          <div class="progress-fill" style:width="{vidSlowStore.pct}%"></div>
        </div>
      {/if}

      <footer class="dock">
        <div class="scale-toggle" role="group" aria-label="Режим">
          <button
            type="button" class="scale-btn"
            class:active={vidSlowStore.mode === "boost"}
            disabled={vidSlowStore.status === "running"}
            onclick={() => (vidSlowStore.mode = "boost")}
            title="Те же N секунд, fps × N (видео выглядит плавнее, аудио сохраняется)"
          >Boost FPS</button>
          <button
            type="button" class="scale-btn"
            class:active={vidSlowStore.mode === "slow"}
            disabled={vidSlowStore.status === "running"}
            onclick={() => (vidSlowStore.mode = "slow")}
            title="Длительность × N, fps без изменений (slow-motion, без аудио)"
          >Slow-mo</button>
        </div>
        <div class="scale-toggle" role="group" aria-label="Кратность">
          {#each [2, 4, 8] as f (f)}
            <button
              type="button"
              class="scale-btn"
              class:active={vidSlowStore.factor === f}
              disabled={vidSlowStore.status === "running"}
              onclick={() => (vidSlowStore.factor = f as 2 | 4 | 8)}
            >×{f}</button>
          {/each}
        </div>
        {#if statusLabel}
          <span class="status">{statusLabel}</span>
        {/if}
        <div class="actions">
          {#if vidSlowStore.errorMsg}
            <span class="error" title={vidSlowStore.errorMsg}>⚠ {vidSlowStore.errorMsg.slice(0, 80)}</span>
          {/if}
          {#if vidSlowStore.input}
            <button class="btn-secondary" onclick={vidSlowClear} disabled={vidSlowStore.status === "running"}>Сбросить</button>
          {/if}
          <button class="btn-secondary" onclick={pickFile} disabled={vidSlowStore.status === "running"}>
            {vidSlowStore.input ? "Сменить" : "Выбрать"}
          </button>
          <button class="btn-primary" onclick={vidSlowRun} disabled={!vidSlowStore.input || vidSlowStore.status === "running"}>
            {vidSlowStore.status === "running" ? "Работа…" : vidSlowStore.mode === "slow" ? "Slow-mo" : "Boost"}
          </button>
        </div>
      </footer>
    </div>
    <SidePanel family="interp" bind:model={vidSlowStore.model} disabled={vidSlowStore.status === "running"} />
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

  .players {
    flex: 1 1 auto; min-height: 0;
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(360px, 1fr));
    gap: var(--space-12); width: 100%;
  }
  .player {
    display: flex; flex-direction: column; gap: 6px; min-height: 0;
    background: var(--surface); border: 1px solid var(--border);
    border-radius: var(--radius-md); padding: 8px;
  }
  .player video {
    width: 100%; flex: 1 1 auto; min-height: 0;
    border-radius: var(--radius-sm); background: #000; object-fit: contain;
  }
  .player-label {
    font: 700 9px / 1 inherit; text-transform: uppercase;
    letter-spacing: 0.6px; color: var(--muted);
  }
  .player-label.out { color: var(--ok); }
  .player-meta {
    font: 500 var(--text-xs) / 1.3 inherit; color: var(--muted);
    overflow: hidden; text-overflow: ellipsis; white-space: nowrap;
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
    flex: 0 0 auto; display: flex; align-items: center;
    gap: var(--space-12); padding: 12px 14px;
  }
  .scale-toggle {
    display: inline-flex; background: var(--surface);
    border: 1px solid var(--border); border-radius: var(--radius-pill);
    padding: 2px;
  }
  .scale-btn {
    appearance: none; border: none; background: transparent;
    color: var(--muted); font: 700 var(--text-xs) / 1 inherit;
    padding: 6px 12px; border-radius: var(--radius-pill); cursor: pointer;
    transition: background 0.15s var(--ease-out), color 0.15s var(--ease-out);
  }
  .scale-btn:hover:not(:disabled):not(.active) { color: var(--fg); }
  .scale-btn.active { background: var(--accent); color: var(--accent-fg); }
  .scale-btn:disabled { opacity: 0.5; cursor: not-allowed; }

  .status {
    font: 500 var(--text-xs) / 1 inherit; color: var(--muted);
    margin-left: var(--space-12); white-space: nowrap;
    overflow: hidden; text-overflow: ellipsis; max-width: 320px;
  }
  .actions { margin-left: auto; display: inline-flex; align-items: center; gap: var(--space-8); }
  .btn-primary, .btn-secondary {
    padding: 8px 16px; border-radius: var(--radius-md);
    border: 1px solid transparent; cursor: pointer;
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
