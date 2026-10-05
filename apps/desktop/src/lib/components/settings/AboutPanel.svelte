<script lang="ts">
  import GlassCard from "$lib/components/common/GlassCard.svelte";
  import Icon from "$lib/components/common/Icon.svelte";
  import Skeleton from "$lib/components/common/Skeleton.svelte";
  import { system } from "$lib/stores/system.svelte";

  const paths = $derived(
    system.info
      ? [
          ["Log", system.info.paths.log_file],
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

<GlassCard title="Atlas Hardware Manager" subtitle={system.info ? `Version ${system.info.version}` : undefined} icon="info-circle" large>
  {#if system.info}
    <dl class="grid grid-cols-[7.5rem_1fr] gap-x-3 gap-y-2 text-[13px]">
      <dt class="text-fg-faint">Platform</dt>
      <dd class="text-fg">{system.info.platform} / {system.info.arch}</dd>
      <dt class="text-fg-faint">Devices</dt>
      <dd class="text-fg">{system.info.simulated ? `Simulated (${system.info.simulated})` : "Real hardware"}</dd>
      {#each paths as [name, path] (name)}
        <dt class="text-fg-faint">{name}</dt>
        <dd class="mono break-all text-[12px] text-fg-muted">{path}</dd>
      {/each}
    </dl>
    {#if system.info.startup_warnings.length > 0}
      <ul class="mt-4 flex flex-col gap-1.5 text-[13px] text-warn-fg">
        {#each system.info.startup_warnings as warning, i (i)}
          <li class="flex items-start gap-2"><Icon name="alert-triangle" size={15} class="mt-0.5" />{warning}</li>
        {/each}
      </ul>
    {/if}
  {:else}
    <div class="flex flex-col gap-2"><Skeleton width="50%" /><Skeleton width="70%" /><Skeleton width="60%" /></div>
  {/if}
</GlassCard>
