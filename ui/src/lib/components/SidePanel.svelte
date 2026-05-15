<script lang="ts">
  // Right-hand sidebar panel: backend rows + per-family model rows.
  // Replaces the old per-tool header. Same row visual language as
  // flov/settings (44px, surface bg, check / Best badge / actions cluster).
  //
  // Backends are shown for ALL options always — disabled rows make it
  // visible to the user that "this backend exists in the architecture
  // but the sidecar wasn't built/found" without hiding capabilities.
  //
  // Models are filtered to `family`; empty list collapses the section.
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { listen } from "@tauri-apps/api/event";

  type ModelInfo = {
    id: string;
    family: string;
    label: string;
    size_mb: number;
    installed: boolean;
    downloading: boolean;
    output_scale: number;
  };
  type BackendState = { choice: string; available: string[] };

  type Props = {
    family: "rembg" | "upscale" | "interp";
    model?: string;
    disabled?: boolean;
  };
  let {
    family,
    model = $bindable<string>(""),
    disabled = false,
  }: Props = $props();

  const BACKENDS = [
    { id: "cuda",   label: "CUDA",     sub: "NVIDIA GPU" },
    { id: "dml",    label: "DirectML", sub: "Любая Windows GPU" },
    { id: "vulkan", label: "Vulkan",   sub: "AMD / Intel iGPU" },
    { id: "coreml", label: "CoreML",   sub: "Apple Silicon" },
    { id: "cpu",    label: "CPU",      sub: "Slow, no GPU" },
  ];
  const PRIORITY = ["cuda", "dml", "vulkan", "coreml", "cpu"];

  let backend = $state<BackendState>({ choice: "auto", available: [] });
  let models = $state<ModelInfo[]>([]);
  let dlPct = $state<Record<string, number>>({});

  const best = $derived(PRIORITY.find((b) => backend.available.includes(b)) ?? "cpu");
  const activeBackend = $derived(backend.choice === "auto" ? best : backend.choice);
  const familyModels = $derived(models.filter((m) => m.family === family));

  async function refreshBackend() {
    try { backend = await invoke<BackendState>("get_backend_state"); } catch {}
  }
  async function refreshModels() {
    try { models = await invoke<ModelInfo[]>("list_models"); } catch {}
  }
  async function pickBackend(id: string) {
    if (disabled || !backend.available.includes(id)) return;
    try { await invoke("set_backend_choice", { choice: id }); await refreshBackend(); }
    catch (e) { alert(String(e)); }
  }
  async function pickModel(id: string, installed: boolean) {
    if (disabled) return;
    model = id;
    if (!installed) await downloadModel(id);
  }
  async function downloadModel(id: string) {
    try { dlPct[id] = 0; await invoke("download_model", { id }); }
    catch (e) { alert(`Не удалось запустить загрузку: ${e}`); }
  }
  async function removeModel(e: MouseEvent, id: string) {
    e.stopPropagation();
    if (!confirm("Удалить модель с диска?")) return;
    try { await invoke("delete_model", { id }); await refreshModels(); } catch {}
  }

  onMount(() => {
    refreshBackend();
    refreshModels();
    const subs = [
      listen<{ id: string; pct: number }>("model-download-progress", (e) => {
        dlPct[e.payload.id] = e.payload.pct;
      }),
      listen<{ id: string }>("model-download-start", () => refreshModels()),
      listen<{ id: string }>("model-download-done", (e) => {
        delete dlPct[e.payload.id];
        refreshModels();
      }),
      listen<{ id: string; error: string }>("model-download-error", (e) => {
        delete dlPct[e.payload.id];
        refreshModels();
      }),
    ];
    return () => { subs.forEach((p) => p.then((f) => f())); };
  });
</script>

<aside class="side">
  <section class="zone">
    <h3 class="zone-title">Backend</h3>
    <ul class="rows">
      {#each BACKENDS as b, i (b.id)}
        {@const enabled = backend.available.includes(b.id)}
        {@const selected = activeBackend === b.id}
        {@const isBest = best === b.id}
        <li
          class="row"
          class:active={selected}
          class:disabled={!enabled}
          style:--i={i}
        >
          <button class="bare" onclick={() => pickBackend(b.id)} disabled={disabled || !enabled} aria-label={b.label}>
            <span class="name">{b.label}</span>
            <span class="size">{b.sub}</span>
            {#if isBest && enabled}
              <span class="badge">Best</span>
            {/if}
            <span class="spacer"></span>
            <span class="check" class:on={selected} aria-hidden="true">
              {#if selected}
                <svg viewBox="0 0 24 24" width="13" height="13" fill="none" stroke="currentColor" stroke-width="2.6" stroke-linecap="round" stroke-linejoin="round"><polyline points="20 6 9 17 4 12"/></svg>
              {/if}
            </span>
          </button>
        </li>
      {/each}
    </ul>
  </section>

  {#if familyModels.length > 0}
    <section class="zone models-zone">
      <h3 class="zone-title">Модели</h3>
      <ul class="rows">
        {#each familyModels as m, i (m.id)}
          {@const downloading = dlPct[m.id] !== undefined || m.downloading}
          {@const pct = dlPct[m.id] ?? 0}
          {@const selected = model === m.id}
          <!-- The model row uses a bare <li> with role=button instead of
               wrapping the whole thing in a <button>. The delete control is
               a real <button>, and nesting buttons is invalid HTML — Svelte
               warns about it because the browser will reparent at hydration. -->
          <!-- svelte-ignore a11y_no_noninteractive_element_to_interactive_role -->
          <!-- svelte-ignore a11y_click_events_have_key_events -->
          <li
            class="row clickable"
            class:active={selected}
            style:--i={i}
            title={m.label}
            role="button"
            tabindex={disabled ? -1 : 0}
            aria-disabled={disabled}
            aria-label={m.label}
            onclick={() => pickModel(m.id, m.installed)}
            onkeydown={(e) => { if (e.key === "Enter" || e.key === " ") { e.preventDefault(); pickModel(m.id, m.installed); } }}
          >
            <span class="name">{m.label}</span>
            <span class="size">{m.size_mb} MB</span>
            <span class="spacer"></span>
            {#if downloading}
              <span class="progress"><span class="bar" style:width="{pct.toFixed(1)}%"></span></span>
              <span class="pct">{pct.toFixed(0)}%</span>
            {:else if m.installed}
              <span class="actions">
                <span class="check" class:on={selected} aria-hidden="true">
                  {#if selected}
                    <svg viewBox="0 0 24 24" width="13" height="13" fill="none" stroke="currentColor" stroke-width="2.6" stroke-linecap="round" stroke-linejoin="round"><polyline points="20 6 9 17 4 12"/></svg>
                  {/if}
                </span>
                <button class="round del-btn" onclick={(e) => removeModel(e, m.id)} aria-label="Удалить">
                  <svg viewBox="0 0 24 24" width="13" height="13" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round"><path d="M3 6h18"/><path d="M19 6l-1 14a2 2 0 0 1-2 2H8a2 2 0 0 1-2-2L5 6"/><path d="M10 11v6"/><path d="M14 11v6"/></svg>
                </button>
              </span>
            {:else}
              <span class="dl-hint">Скачать</span>
            {/if}
          </li>
        {/each}
      </ul>
    </section>
  {/if}
</aside>

<style>
  .side {
    flex: 0 0 260px;
    min-width: 0;
    min-height: 0;
    display: flex;
    flex-direction: column;
    gap: var(--space-16);
    padding: 0 14px 12px 0;
    overflow-y: auto;
  }
  .zone {
    display: flex;
    flex-direction: column;
    min-height: 0;
  }
  .zone-title {
    margin: 0 0 var(--space-8) 4px;
    font: 600 var(--text-xs) / 1 inherit;
    color: var(--muted);
    text-transform: uppercase;
    letter-spacing: 0.6px;
  }

  ul.rows {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
  }

  li.row {
    height: 44px;
    border-radius: var(--radius-md);
    background: var(--surface);
    transition: background 0.15s var(--ease-out), color 0.15s var(--ease-out);
    color: var(--fg);
    animation: rowIn 0.25s var(--ease-out) both;
    animation-delay: calc(var(--i) * 18ms);
  }
  /* Model rows are click-to-select, so the whole row needs cursor + padding
     because there's no inner <button> filling it (would be nested-button). */
  li.row.clickable {
    cursor: pointer;
    padding: 0 12px;
    display: flex;
    align-items: center;
    gap: var(--space-8);
    user-select: none;
  }
  li.row.clickable:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 2px;
  }
  li.row.clickable[aria-disabled="true"] { pointer-events: none; opacity: 0.6; }
  @keyframes rowIn {
    from { opacity: 0; transform: translateY(4px); }
    to   { opacity: 1; transform: translateY(0); }
  }
  li.row:hover { background: var(--hover); }
  li.row.active { background: var(--accent); color: var(--accent-fg); }
  li.row.active:hover { background: var(--accent); }
  li.row.disabled { pointer-events: none; color: var(--muted); }
  li.row.disabled .name, li.row.disabled .size { opacity: 0.55; }
  li.row.disabled .check { opacity: 0.4; background: transparent; }

  button.bare {
    width: 100%;
    height: 100%;
    appearance: none;
    background: transparent !important;
    border: none !important;
    padding: 0 12px !important;
    border-radius: inherit !important;
    color: inherit;
    font: inherit;
    text-align: left;
    cursor: pointer;
    display: flex;
    align-items: center;
    gap: var(--space-8);
  }
  li.row.disabled button.bare { cursor: not-allowed; }

  .name {
    font-weight: 600;
    font-size: var(--text-sm);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    flex: 0 1 auto;
    min-width: 0;
  }
  .size { font: 500 var(--text-xs) / 1 inherit; opacity: 0.7; flex-shrink: 0; }
  .spacer { flex: 1 1 auto; }

  .badge {
    font: 700 9px / 1 inherit;
    text-transform: uppercase;
    letter-spacing: 0.6px;
    padding: 4px 8px;
    border-radius: var(--radius-pill);
    background: color-mix(in srgb, currentColor 14%, transparent);
    color: inherit;
    flex-shrink: 0;
  }
  li.row.active .badge {
    background: color-mix(in srgb, var(--accent-fg) 22%, transparent);
    color: var(--accent-fg);
  }

  .check {
    width: 26px;
    height: 26px;
    border-radius: 50%;
    border: 1px solid var(--border);
    background: var(--bg-elevated);
    color: var(--fg);
    display: inline-flex;
    align-items: center;
    justify-content: center;
    flex-shrink: 0;
    transition: background 0.15s var(--ease-out), color 0.15s var(--ease-out), border-color 0.15s var(--ease-out);
  }
  li.row.active .check {
    background: var(--accent-fg);
    color: var(--accent);
    border-color: transparent;
  }

  /* Models row extras */
  .progress {
    width: 70px;
    height: 4px;
    border-radius: var(--radius-pill);
    background: color-mix(in srgb, currentColor 18%, transparent);
    overflow: hidden;
    flex-shrink: 0;
  }
  .progress .bar { display: block; height: 100%; background: var(--accent); transition: width 0.3s var(--ease-out); }
  li.row.active .progress .bar { background: var(--accent-fg); }
  .pct {
    font: 500 var(--text-xs) / 1 inherit;
    opacity: 0.75;
    font-variant-numeric: tabular-nums;
    min-width: 30px;
    text-align: right;
    flex-shrink: 0;
  }
  .dl-hint {
    font: 700 9px / 1 inherit;
    text-transform: uppercase;
    letter-spacing: 0.6px;
    padding: 4px 8px;
    border-radius: var(--radius-pill);
    background: var(--accent);
    color: var(--accent-fg);
    flex-shrink: 0;
  }

  .actions { display: inline-flex; align-items: center; gap: 6px; flex-shrink: 0; }
  button.round {
    appearance: none;
    width: 26px;
    height: 26px;
    border-radius: 50%;
    border: 1px solid var(--border);
    background: var(--bg-elevated);
    color: var(--fg);
    cursor: pointer;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    padding: 0;
    transition: background 0.15s var(--ease-out), color 0.15s var(--ease-out),
                transform 0.2s var(--ease-out), opacity 0.18s var(--ease-out),
                border-color 0.15s var(--ease-out), margin-left 0.22s var(--ease-out);
  }
  /* delete slides in on row hover (matches flov pattern) */
  button.del-btn {
    opacity: 0;
    margin-left: -32px;
    pointer-events: none;
    transform: scale(0.7);
    color: var(--muted);
  }
  li.row:hover button.del-btn {
    opacity: 1;
    margin-left: 0;
    transform: scale(1);
    pointer-events: auto;
  }
  button.del-btn:hover:not(:disabled) {
    background: var(--danger) !important;
    border-color: var(--danger) !important;
    color: #ffffff !important;
  }
  li.row.active button.del-btn {
    color: var(--accent-fg);
    background: transparent;
    border-color: color-mix(in srgb, var(--accent-fg) 30%, transparent);
  }
</style>
