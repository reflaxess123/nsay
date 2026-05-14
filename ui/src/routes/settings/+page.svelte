<script lang="ts">
  import Backend from "$lib/settings/Backend.svelte";
  import Models from "$lib/settings/Models.svelte";
</script>

<div class="page">
  <section class="grid">
    <div class="card">
      <h2>Backend</h2>
      <Backend />
    </div>
    <div class="card">
      <h2>Models</h2>
      <Models />
    </div>
  </section>
</div>

<style>
  .page {
    flex: 1 1 auto;
    min-height: 0;
    overflow: auto;
    padding: var(--space-16);
  }
  .grid {
    display: grid;
    grid-template-columns: minmax(0, 1fr) minmax(0, 1fr);
    gap: var(--space-16);
    height: 100%;
  }
  .card {
    background: var(--bg-elevated);
    border: 1px solid var(--border);
    border-radius: var(--radius-lg);
    padding: var(--space-16);
    box-shadow: var(--shadow-card);
    overflow: auto;
    min-height: 0;
  }
  .card h2 {
    margin: 0 0 var(--space-12);
    font: 600 var(--text-base) / 1.2 inherit;
  }
</style>
