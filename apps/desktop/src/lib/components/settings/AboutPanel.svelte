<script lang="ts">
  import Panel from "$lib/components/common/Panel.svelte";
  import { system } from "$lib/stores/system.svelte";

  const paths = $derived(
    system.info
      ? [
          ["Data", system.info.paths.data_dir],
          ["Cache", system.info.paths.cache_dir],
          ["Settings", system.info.paths.settings_file],
          ["Inventory", system.info.paths.inventory_file],
          ["Releases", system.info.paths.releases_file],
          ["Release cache", system.info.paths.release_cache_dir],
        ]
      : [],
  );
</script>

<Panel eyebrow="About" title="Atlas Hardware Manager">
  {#if system.info}
    <dl class="grid grid-cols-[7rem_1fr] gap-y-1 text-xs">
      <dt class="text-surface-400">Version</dt>
      <dd class="font-mono text-surface-50">{system.info.version}</dd>
      <dt class="text-surface-400">Platform</dt>
      <dd class="text-surface-100">{system.info.platform} / {system.info.arch}</dd>
      <dt class="text-surface-400">Devices</dt>
      <dd class="text-surface-100">{system.info.simulated ? `simulated (${system.info.simulated})` : "real hardware"}</dd>
      {#each paths as [name, path] (name)}
        <dt class="text-surface-400">{name}</dt>
        <dd class="break-all font-mono text-[0.68rem] text-surface-200">{path}</dd>
      {/each}
    </dl>
    {#if system.info.startup_warnings.length > 0}
      <p class="micro-label mb-1 mt-4">Startup warnings</p>
      <ul class="flex flex-col gap-1 text-xs text-warning-200">
        {#each system.info.startup_warnings as warning, i (i)}
          <li><i class="fa-solid fa-triangle-exclamation mr-1" aria-hidden="true"></i>{warning}</li>
        {/each}
      </ul>
    {/if}
  {:else}
    <p class="text-xs text-surface-400">Loading…</p>
  {/if}
</Panel>
