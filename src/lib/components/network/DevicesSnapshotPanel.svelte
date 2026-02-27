<script lang="ts">
  import { Panel, UsageTile } from "$lib/components";
  import type { NetworkSnapshotMetric } from "./types";

  export let metrics: NetworkSnapshotMetric[] = [];
</script>

<Panel eyebrow="Network Snapshot" subtitle="Core metrics for Atlas Layer 2/3 infrastructure.">
  <div class="grid gap-4 md:grid-cols-2 xl:grid-cols-4">
    {#each metrics as metric (metric.label)}
      <UsageTile
        label={metric.label}
        value={metric.value}
        unit={metric.unit}
        accent={metric.accent}
      />
    {/each}
  </div>
</Panel>
