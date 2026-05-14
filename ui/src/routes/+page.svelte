<script lang="ts">
  import { goto } from "$app/navigation";
  import { onMount } from "svelte";
  // Home doesn't really exist in nsay — bounce straight to Rembg.
  onMount(() => { goto("/rembg", { replaceState: true }); });
</script>
