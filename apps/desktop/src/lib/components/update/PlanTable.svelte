<script lang="ts">
  import type { JobPlan, PlannedDevice } from "#lib/api/client.ts";
  import { keyString } from "#lib/api/client.ts";
  import Icon from "#lib/components/common/Icon.svelte";
  import Pill from "#lib/components/common/Pill.svelte";
  import type { Tone } from "#lib/format.ts";

  let { plan }: { plan: JobPlan } = $props();

  function order(device: PlannedDevice): { text: string; tone: Tone } {
    if (device.canary) return { text: "Goes first", tone: "warning" };
    if (device.plan.concurrency.kind === "exclusive") return { text: "One at a time", tone: "info" };
    return { text: "In parallel", tone: "neutral" };
  }
</script>

<ul class="flex flex-col gap-1.5">
  {#each plan.devices as device (keyString(device.device))}
    {@const o = order(device)}
    <li class="glass flex flex-col gap-1 px-3.5 py-2.5">
      <div class="flex items-center justify-between gap-3">
        <span class="truncate text-[13px] font-medium text-fg">{device.name}</span>
        <Pill tone={o.tone} label={o.text} title={device.plan.concurrency.kind === "exclusive" ? device.plan.concurrency.resource : undefined} />
      </div>
      <div class="flex items-center gap-1.5 text-[12px]">
        <span class="mono text-fg-faint">{device.from_version ?? "?"}</span>
        <Icon name="arrow-right" size={12} class="text-fg-faint" />
        <span class="mono truncate text-fg">{device.release.version}</span>
      </div>
      <p class="text-[12px] text-fg-muted">{device.plan.summary}</p>
    </li>
  {/each}
</ul>
