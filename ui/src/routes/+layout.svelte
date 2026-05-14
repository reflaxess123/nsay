<script lang="ts">
  import { page } from "$app/stores";
  import { getCurrentWindow } from "@tauri-apps/api/window";

  type Props = { children?: import("svelte").Snippet };
  let { children }: Props = $props();

  const tabs = [
    { id: "rembg",    label: "Rembg",    href: "/rembg",    sub: "Удаление фона" },
    { id: "upscale",  label: "Upscale",  href: "/upscale",  sub: "Апскейл" },
    { id: "interp",   label: "Interp",   href: "/interp",   sub: "Интерполяция" },
    { id: "settings", label: "Settings", href: "/settings", sub: "Настройки" },
  ];

  const currentTab = $derived.by(() => {
    const path = $page.url.pathname.replace(/\/+$/, "") || "/";
    return tabs.find((t) => path === t.href || path.startsWith(t.href + "/"))?.id ?? "rembg";
  });

  const win = getCurrentWindow();
  function close() { win.hide(); }
  function minimize() { win.minimize(); }
  function maximize() { win.toggleMaximize(); }
</script>

<div class="app-shell">
  <!-- Drag strip with custom window controls. Frameless window: only
       elements explicitly tagged data-tauri-drag-region accept drag. -->
  <div class="drag-strip" data-tauri-drag-region>
    <span class="brand" data-tauri-drag-region>
      <span class="brand-name">nsay</span>
      <span class="brand-by">local AI toolkit</span>
    </span>

    <div class="tabs-wrap">
      <nav class="tabs" data-tauri-drag-region={false}>
        {#each tabs as t (t.id)}
          <a
            href={t.href}
            class="tab"
            class:active={currentTab === t.id}
            data-sveltekit-preload-data="hover"
          >
            <span class="tab-label">{t.label}</span>
          </a>
        {/each}
      </nav>
    </div>

    <div class="winctl">
      <button class="wbtn" onclick={minimize} aria-label="Minimize">
        <svg viewBox="0 0 16 16" width="12" height="12" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round"><path d="M3 8 L13 8"/></svg>
      </button>
      <button class="wbtn" onclick={maximize} aria-label="Maximize">
        <svg viewBox="0 0 16 16" width="11" height="11" fill="none" stroke="currentColor" stroke-width="1.4"><rect x="3.5" y="3.5" width="9" height="9" rx="1.5"/></svg>
      </button>
      <button class="wbtn close-x" onclick={close} aria-label="Close">
        <svg viewBox="0 0 16 16" width="14" height="14" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round"><path d="M4 4 L12 12 M12 4 L4 12"/></svg>
      </button>
    </div>
  </div>

  <main class="content">
    {@render children?.()}
  </main>
</div>

<style>
  /* ===== TOKENS (copy of flov design system) ===== */
  :global(:root) {
    --bg: #f3f3f6;
    --bg-elevated: #ffffff;
    --surface: #f8f9fa;
    --fg: #18181b;
    --muted: #71717a;
    --border: #e4e4e7;
    --accent: #18181b;
    --accent-fg: #ffffff;
    --accent-soft: rgba(24, 24, 27, 0.06);
    --ok: #10b981;
    --warn: #f59e0b;
    --danger: #ef4444;
    --hover: #ececf0;
    --pressed: #e4e4e7;
    --shadow-card: 0 1px 2px rgba(0, 0, 0, 0.04), 0 4px 12px -4px rgba(0, 0, 0, 0.05);

    --text-xs: 12px;
    --text-sm: 13px;
    --text-base: 15px;
    --text-lg: 18px;
    --text-xl: 22px;
    --text-display: 30px;

    --space-4: 4px;
    --space-8: 8px;
    --space-12: 12px;
    --space-16: 16px;
    --space-20: 20px;
    --space-24: 24px;
    --space-32: 32px;

    --radius-sm: 8px;
    --radius-md: 12px;
    --radius-lg: 16px;
    --radius-xl: 20px;
    --radius-pill: 999px;

    --ease-out: cubic-bezier(0.2, 0.8, 0.2, 1);
    --ease-spring: cubic-bezier(0.34, 1.56, 0.64, 1);
  }

  @media (prefers-color-scheme: dark) {
    :global(:root) {
      --bg: #0e0e11;
      --bg-elevated: #18181b;
      --surface: #232327;
      --fg: #fafafa;
      --muted: #a1a1aa;
      --border: #2e2e33;
      --accent: #d9ff42;
      --accent-fg: #0a0a0c;
      --accent-soft: rgba(217, 255, 66, 0.12);
      --ok: #d9ff42;
      --warn: #fbbf24;
      --danger: #f87171;
      --hover: #2d2d33;
      --pressed: #3f3f46;
      --shadow-card: 0 1px 2px rgba(0, 0, 0, 0.4), 0 4px 16px -4px rgba(0, 0, 0, 0.4);
    }
  }

  :global(html, body) {
    margin: 0;
    padding: 0;
    height: 100%;
    background: transparent !important;
    color: var(--fg);
    font-family:
      -apple-system, BlinkMacSystemFont, "Segoe UI", Inter, system-ui, sans-serif;
    -webkit-font-smoothing: antialiased;
    overflow: hidden;
    font-size: var(--text-sm);
  }

  :global(::-webkit-scrollbar) { width: 6px; height: 6px; }
  :global(::-webkit-scrollbar-track) { background: transparent; }
  :global(::-webkit-scrollbar-thumb) { background: var(--border); border-radius: 99px; }
  :global(::-webkit-scrollbar-thumb:hover) { background: var(--muted); }

  /* ===== APP SHELL ===== */
  .app-shell {
    position: fixed;
    inset: 0;
    display: flex;
    flex-direction: column;
    background: var(--bg);
    border-radius: var(--radius-lg);
    overflow: hidden;
    border: 1px solid var(--border);
    animation: appear 0.3s var(--ease-out);
  }
  @keyframes appear {
    from { opacity: 0; transform: scale(0.985); }
    to   { opacity: 1; transform: scale(1); }
  }

  /* ===== DRAG STRIP — brand left, tabs centre, controls right.
     Tabs are absolutely centred to the window so the brand/winctl
     widths don't shift them off-axis. ===== */
  .drag-strip {
    flex: 0 0 auto;
    position: relative;
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 12px 14px;
    background: transparent;
  }
  .tabs-wrap {
    position: absolute;
    left: 50%;
    top: 50%;
    transform: translate(-50%, -50%);
  }

  .brand {
    display: inline-flex;
    align-items: baseline;
    gap: 6px;
    pointer-events: none;
  }
  .brand-name {
    font: 700 14px / 1 inherit;
    color: var(--fg);
    letter-spacing: -0.2px;
  }
  .brand-by {
    font: 500 11px / 1 inherit;
    color: var(--muted);
  }

  .tabs {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: var(--space-4);
    background: var(--surface);
    padding: 4px;
    border-radius: var(--radius-pill);
    border: 1px solid var(--border);
  }
  .tab {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 6px 14px;
    border-radius: var(--radius-pill);
    color: var(--muted);
    text-decoration: none;
    font: 500 var(--text-sm) / 1 inherit;
    transition: background 0.15s var(--ease-out), color 0.15s var(--ease-out);
  }
  .tab:hover { color: var(--fg); background: var(--hover); }
  .tab.active {
    background: var(--accent);
    color: var(--accent-fg);
  }

  .winctl {
    display: inline-flex;
    align-items: center;
    justify-content: flex-end;
    gap: 2px;
  }
  .wbtn {
    width: 30px;
    height: 28px;
    border: none;
    border-radius: var(--radius-sm);
    background: transparent;
    color: var(--muted);
    display: inline-flex;
    align-items: center;
    justify-content: center;
    cursor: pointer;
    padding: 0;
    transition: background-color 0.15s var(--ease-out), color 0.15s var(--ease-out);
  }
  .wbtn:hover { background: var(--hover); color: var(--fg); }
  .close-x:hover { background: var(--danger); color: #fff; }

  /* ===== CONTENT — each route owns its layout =====
     No padding here: routes that want full-bleed (rembg/upscale dropzones)
     get to grow edge-to-edge; routes that want gutters add their own. */
  .content {
    flex: 1 1 auto;
    min-height: 0;
    overflow: hidden;
    display: flex;
    flex-direction: column;
  }
</style>
