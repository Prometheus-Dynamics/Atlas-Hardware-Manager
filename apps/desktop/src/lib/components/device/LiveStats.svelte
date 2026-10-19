<script lang="ts">
  // Live readings for one device, polled while shown.
  import { keyString, type DeviceRecord } from "#lib/api/client.ts";
  import Icon from "#lib/components/common/Icon.svelte";
  import Skeleton from "#lib/components/common/Skeleton.svelte";
  import { coreMetrics, isCore, sortMetrics } from "#lib/metrics.ts";
  import { live, watchLive } from "#lib/stores/live.svelte.ts";
  import { rise, stagger } from "#lib/ui/motion.ts";
  import MetricTile from "./MetricTile.svelte";

  let { record }: { record: DeviceRecord } = $props();

  const id = $derived(keyString(record.key));
  const all = $derived(live.metrics.get(id) ?? []);
  // Each core shows inside the CPU tile, not as a tile of its own.
  const metrics = $derived(sortMetrics(all.filter((m) => !isCore(m))));
  const cores = $derived(coreMetrics(all));
  const error = $derived(live.errors.get(id));

  $effect(() => watchLive([record]));
</script>

<section class="flex flex-col gap-2.5">
  <h3 class="flex items-center gap-2 text-[12px] font-medium uppercase tracking-[0.06em] text-fg-faint">
    <span class="live-dot"></span>Live
  </h3>
  {#if metrics.length === 0 && error}
    <p class="flex items-center gap-2 text-[13px] text-warn-fg"><Icon name="alert-triangle" size={15} />{error}</p>
  {:else if metrics.length === 0}
    <div class="tiles">
      {#each [0, 1, 2, 3, 4, 5] as i (i)}<Skeleton height={100} />{/each}
    </div>
  {:else}
    <div class="tiles">
      {#each metrics as metric, i (metric.id)}
        <div in:rise={{ delay: stagger(i) }}>
          <MetricTile {metric} series={live.series(id, metric.id)} times={live.seriesTimes(id)} cores={metric.id === "cpu" ? cores : []} />
        </div>
      {/each}
    </div>
  {/if}
</section>

<style>
  .tiles {
    display: grid;
    gap: 10px;
    grid-template-columns: repeat(auto-fill, minmax(190px, 1fr));
  }
  .live-dot {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: var(--ok);
  }
</style>
