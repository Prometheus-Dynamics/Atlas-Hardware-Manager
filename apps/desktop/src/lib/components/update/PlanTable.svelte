<script lang="ts">
  // What each device will do, quietly: one line per device.
  import type { JobPlan, PlannedDevice } from "#lib/api/client.ts";
  import { keyString } from "#lib/api/client.ts";
  import Icon from "#lib/components/common/Icon.svelte";
  import Pill from "#lib/components/common/Pill.svelte";

  let { plan }: { plan: JobPlan } = $props();

  function order(device: PlannedDevice): string | null {
    if (device.canary) return null;
    if (device.plan.concurrency.kind === "exclusive") return "one at a time";
    return null;
  }
</script>

<ul class="list">
  {#each plan.devices as device (keyString(device.device))}
    {@const o = order(device)}
    <li class="flex flex-col gap-0.5 py-2">
      <div class="flex items-center gap-2">
        <span class="min-w-0 flex-1 truncate text-[13px] text-fg">{device.name}</span>
        <span class="flex shrink-0 items-center gap-1 text-[12px]">
          <span class="mono text-fg-faint">{device.from_version ?? "?"}</span>
          <Icon name="arrow-right" size={12} class="text-fg-faint" />
          <span class="mono text-fg">{device.release.version}</span>
        </span>
        {#if device.canary}<Pill tone="warning" label="Goes first" />{/if}
      </div>
      <p
        class="truncate text-[12px] text-fg-faint"
        title={device.plan.concurrency.kind === "exclusive" ? device.plan.concurrency.resource : undefined}
      >
        {device.plan.summary}{o ? ` · ${o}` : ""}
      </p>
    </li>
  {/each}
</ul>

<style>
  .list > li + li {
    border-top: 1px solid var(--hairline);
  }
</style>
