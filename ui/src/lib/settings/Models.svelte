<script lang="ts">
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
    local_path: string | null;
  };

  let models = $state<ModelInfo[]>([]);
  // id → 0..100, set by model-download-progress events.
  let progress = $state<Record<string, number>>({});

  async function refresh() {
    models = await invoke<ModelInfo[]>("list_models");
  }

  async function download(id: string) {
    try {
      progress[id] = 0;
      await invoke("download_model", { id });
      // download_model spawns in a background thread; refresh will be
      // re-triggered by model-download-done / -error.
    } catch (e) {
      alert(`Не удалось запустить загрузку: ${e}`);
    }
  }

  async function remove(id: string) {
    if (!confirm("Удалить модель с диска?")) return;
    try {
      await invoke("delete_model", { id });
      await refresh();
    } catch (e) {
      alert(`Не удалось удалить: ${e}`);
    }
  }

  // Group by family for display.
  const groups = $derived.by(() => {
    const order = ["rembg", "upscale", "interp"];
    const map: Record<string, ModelInfo[]> = {};
    for (const m of models) {
      (map[m.family] ??= []).push(m);
    }
    return order
      .filter((f) => map[f]?.length)
      .map((f) => ({ family: f, items: map[f] }));
  });

  function familyLabel(f: string): string {
    return { rembg: "Удаление фона", upscale: "Апскейл", interp: "Интерполяция" }[f] ?? f;
  }

  onMount(() => {
    refresh();
    const u = [
      listen<{ id: string; pct: number }>("model-download-progress", (e) => {
        progress[e.payload.id] = e.payload.pct;
      }),
      listen<{ id: string }>("model-download-start", (_e) => { refresh(); }),
      listen<{ id: string }>("model-download-done", (e) => {
        delete progress[e.payload.id];
        refresh();
      }),
      listen<{ id: string; error: string }>("model-download-error", (e) => {
        delete progress[e.payload.id];
        refresh();
        alert(`Скачивание ${e.payload.id} не удалось:\n${e.payload.error}`);
      }),
    ];
    return () => { u.forEach((p) => p.then((f) => f())); };
  });
</script>

<div class="models">
  {#each groups as g (g.family)}
    <section class="group">
      <h3 class="group-title">{familyLabel(g.family)}</h3>
      <ul class="rows">
        {#each g.items as m (m.id)}
          {@const pct = progress[m.id] ?? 0}
          <li class="row" class:installed={m.installed}>
            <span class="name">{m.label}</span>
            <span class="size">{m.size_mb} MB</span>
            <span class="spacer"></span>
            {#if m.downloading}
              <div class="progress" aria-label="Скачивание">
                <div class="bar" style:width="{pct}%"></div>
                <span class="progress-text">{pct}%</span>
              </div>
            {:else if m.installed}
              <span class="badge ok">Установлено</span>
              <button class="bare-btn danger" onclick={() => remove(m.id)} aria-label="Удалить">
                <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                  <path d="M3 6h18"/><path d="M8 6V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2"/><path d="M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6"/>
                </svg>
              </button>
            {:else}
              <button class="btn" onclick={() => download(m.id)}>Скачать</button>
            {/if}
          </li>
        {/each}
      </ul>
    </section>
  {/each}
</div>

<style>
  .models { display: flex; flex-direction: column; gap: var(--space-16); }

  .group-title {
    margin: 0 0 var(--space-8);
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
    padding: 0 14px;
    border-radius: var(--radius-md);
    background: var(--surface);
    color: var(--fg);
    display: flex;
    align-items: center;
    gap: var(--space-12);
    transition: background 0.15s var(--ease-out);
  }
  li.row:hover { background: var(--hover); }

  .name { font-weight: 600; font-size: var(--text-sm); flex-shrink: 0; }
  .size { font: 500 var(--text-xs) / 1 inherit; color: var(--muted); flex-shrink: 0; }
  .spacer { flex: 1 1 auto; }

  .badge {
    font: 700 9px / 1 inherit;
    text-transform: uppercase;
    letter-spacing: 0.6px;
    padding: 4px 8px;
    border-radius: var(--radius-pill);
    color: inherit;
    flex-shrink: 0;
    background: color-mix(in srgb, currentColor 14%, transparent);
  }
  .badge.ok { color: var(--ok); }

  .btn {
    appearance: none;
    border: 1px solid var(--border);
    background: var(--bg-elevated);
    color: var(--fg);
    border-radius: var(--radius-md);
    padding: 6px 12px;
    font: 600 var(--text-xs) / 1 inherit;
    cursor: pointer;
    transition: background 0.15s var(--ease-out), color 0.15s var(--ease-out), border-color 0.15s var(--ease-out);
    flex-shrink: 0;
  }
  .btn:hover { background: var(--accent); color: var(--accent-fg); border-color: var(--accent); }

  .bare-btn {
    appearance: none;
    border: none;
    background: transparent;
    color: var(--muted);
    width: 28px;
    height: 28px;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    border-radius: var(--radius-sm);
    cursor: pointer;
    transition: background 0.15s var(--ease-out), color 0.15s var(--ease-out);
    flex-shrink: 0;
  }
  .bare-btn:hover { background: var(--hover); color: var(--fg); }
  .bare-btn.danger:hover { background: var(--danger); color: #fff; }

  .progress {
    position: relative;
    width: 160px;
    height: 22px;
    background: var(--bg-elevated);
    border: 1px solid var(--border);
    border-radius: var(--radius-pill);
    overflow: hidden;
    flex-shrink: 0;
  }
  .bar {
    position: absolute;
    inset: 0 auto 0 0;
    background: var(--accent);
    transition: width 0.15s var(--ease-out);
  }
  .progress-text {
    position: absolute;
    inset: 0;
    display: flex;
    align-items: center;
    justify-content: center;
    font: 700 11px / 1 inherit;
    color: var(--fg);
    mix-blend-mode: difference;
    color: #fff;
  }
</style>
