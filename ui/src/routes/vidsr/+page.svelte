<script lang="ts">
  import { listen } from "@tauri-apps/api/event";
  import { open as openDialog } from "@tauri-apps/plugin-dialog";
  import { onMount } from "svelte";
  import SidePanel from "$lib/components/SidePanel.svelte";
  import {
    vidsrStore,
    vidsrInit,
    vidsrSetInput,
    vidsrClear,
    vidsrRun,
  } from "$lib/state/vidsr.svelte.ts";

  async function pickFile() {
    const f = await openDialog({
      multiple: false,
      filters: [{ name: "Video", extensions: ["mp4", "mov", "mkv", "webm", "avi"] }],
    });
    if (typeof f === "string") vidsrSetInput(f);
  }

  onMount(() => {
    vidsrInit();
    const u = listen<{ paths: string[] }>("tauri://drag-drop", (e) => {
      if (e.payload.paths?.length) vidsrSetInput(e.payload.paths[0]);
    });
    return () => { u.then((f) => f()); };
  });

  const fname = $derived.by(() => {
    const p = vidsrStore.input;
    if (!p) return "";
    return p.split(/[\\/]/).pop() ?? p;
  });
  const statusLabel = $derived.by(() => {
    const s = vidsrStore.status;
    if (s === "running") {
      const p = vidsrStore.probe;
      const f = vidsrStore.frame;
      const t = p?.total_frames ?? 0;
      return `${vidsrStore.pct}% · кадр ${f}${t ? `/${t}` : ""}`;
    }
    if (s === "done") return `Готово ×${vidsrStore.scale} · ${vidsrStore.probe?.encoder ?? ""}`;
    if (s === "error") return "Ошибка";
    return "";
  });
</script>

<div class="page">
  <div class="body">
    <div class="work">
      <section class="stage">
        {#if !vidsrStore.input}
          <button class="empty" onclick={pickFile}>
            <svg viewBox="0 0 24 24" width="44" height="44" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round">
              <path d="M12 3v13"/><path d="m7 12 5 5 5-5"/><path d="M5 21h14"/>
            </svg>
            <span class="empty-title">Перетащи видео сюда</span>
            <span class="empty-sub">RealBasicVSR · temporal-aware · CUDA only</span>
          </button>
        {:else}
          <div class="players">
            <div class="player">
              <div class="player-label">Источник</div>
              <video src={vidsrStore.inputUrl} controls muted playsinline></video>
              <div class="player-meta">
                {fname}
                {#if vidsrStore.probe}
                  · {vidsrStore.probe.src_w}×{vidsrStore.probe.src_h}
                {/if}
              </div>
            </div>
            {#if vidsrStore.outputUrl}
              <div class="player">
                <div class="player-label out">Результат</div>
                {#key vidsrStore.bust}
                  <video src={vidsrStore.outputUrl} controls muted playsinline></video>
                {/key}
                <div class="player-meta">
                  {#if vidsrStore.probe}
                    {vidsrStore.probe.out_w}×{vidsrStore.probe.out_h} · {vidsrStore.probe.encoder}
                  {/if}
                </div>
              </div>
            {/if}
          </div>
        {/if}
      </section>

      {#if vidsrStore.status === "running"}
        <div class="progress-strip">
          <div class="progress-fill" style:width="{vidsrStore.pct}%"></div>
        </div>
      {/if}

      <footer class="dock">
        <div class="scale-toggle" role="group" aria-label="Множитель">
          {#each [2, 3, 4] as s (s)}
            <button
              type="button"
              class="scale-btn"
              class:active={vidsrStore.scale === s}
              disabled={vidsrStore.status === "running"}
              onclick={() => (vidsrStore.scale = s as 2 | 3 | 4)}
            >×{s}</button>
          {/each}
        </div>
        <label class="window-input" title="Размер клипа который сидекар подаёт модели за один forward pass. RealBasicVSR (libtorch). Выше = плавнее, но линейно больше VRAM.">
          <span>window</span>
          <input
            type="number"
            min="3"
            max="30"
            step="1"
            disabled={vidsrStore.status === "running"}
            bind:value={vidsrStore.window}
          />
        </label>
        <!-- FlashVSR-Pro (docker backend) tuning. libtorch sidecars ignore. -->
        <div class="diff-toggle" role="group" aria-label="FlashVSR режим" title="FlashVSR-Pro (docker). tiny — 8GB VRAM, full — 12+GB.">
          {#each ["tiny", "full"] as m (m)}
            <button
              type="button"
              class="diff-btn"
              class:active={vidsrStore.mode === m}
              disabled={vidsrStore.status === "running"}
              onclick={() => (vidsrStore.mode = m as "tiny" | "full")}
            >{m}</button>
          {/each}
        </div>
        <label class="chk" title="Tile VAE — обязательно для 12GB GPU."><input type="checkbox" disabled={vidsrStore.status === "running"} bind:checked={vidsrStore.tileVae} /> tile-vae</label>
        <label class="chk" title="Tile DiT — снижает peak VRAM."><input type="checkbox" disabled={vidsrStore.status === "running"} bind:checked={vidsrStore.tileDit} /> tile-dit</label>
        <label class="chk" title="Сохранить аудио из источника."><input type="checkbox" disabled={vidsrStore.status === "running"} bind:checked={vidsrStore.keepAudio} /> audio</label>
        {#if statusLabel}
          <span class="status">{statusLabel}</span>
        {/if}
        <div class="actions">
          {#if vidsrStore.errorMsg}
            <span class="error" title={vidsrStore.errorMsg}>⚠ {vidsrStore.errorMsg.slice(0, 80)}</span>
          {/if}
          {#if vidsrStore.input}
            <button class="btn-secondary" onclick={vidsrClear} disabled={vidsrStore.status === "running"}>Сбросить</button>
          {/if}
          <button class="btn-secondary" onclick={pickFile} disabled={vidsrStore.status === "running"}>
            {vidsrStore.input ? "Сменить" : "Выбрать"}
          </button>
          <button class="btn-primary" onclick={vidsrRun} disabled={!vidsrStore.input || vidsrStore.status === "running"}>
            {vidsrStore.status === "running" ? "Работа…" : "VSR"}
          </button>
        </div>
      </footer>
    </div>
    <SidePanel family="vidsr" bind:model={vidsrStore.model} disabled={vidsrStore.status === "running"} />
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

  .window-input {
    display: inline-flex; align-items: center; gap: 6px;
    font: 500 var(--text-xs) / 1 inherit; color: var(--muted);
  }
  .window-input input {
    width: 52px; padding: 6px 8px;
    background: var(--surface); border: 1px solid var(--border);
    border-radius: var(--radius-md); color: var(--fg);
    font: 600 var(--text-xs) / 1 inherit;
    -moz-appearance: textfield;
  }
  .window-input input::-webkit-inner-spin-button { -webkit-appearance: none; }
  .window-input input:focus { outline: 1px solid var(--accent); }

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

  .diff-toggle {
    display: inline-flex; background: var(--surface);
    border: 1px solid var(--border); border-radius: var(--radius-pill);
    padding: 2px;
  }
  .diff-btn {
    appearance: none; border: none; background: transparent;
    color: var(--muted); font: 700 var(--text-xs) / 1 inherit;
    padding: 6px 12px; border-radius: var(--radius-pill); cursor: pointer;
    text-transform: uppercase; letter-spacing: 0.4px;
    transition: background 0.15s var(--ease-out), color 0.15s var(--ease-out);
  }
  .diff-btn:hover:not(:disabled):not(.active) { color: var(--fg); }
  .diff-btn.active { background: var(--accent); color: var(--accent-fg); }
  .diff-btn:disabled { opacity: 0.5; cursor: not-allowed; }

  .chk {
    display: inline-flex; align-items: center; gap: 4px;
    font: 500 var(--text-xs) / 1 inherit; color: var(--muted);
    cursor: pointer; user-select: none;
  }
  .chk input { accent-color: var(--accent); margin: 0; }
</style>
