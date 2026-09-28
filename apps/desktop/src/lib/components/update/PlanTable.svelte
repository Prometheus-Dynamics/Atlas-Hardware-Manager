<script lang="ts">
  import type { JobPlan, PlannedDevice } from "$lib/api/client";
  import { keyString } from "$lib/api/client";

  let { plan }: { plan: JobPlan } = $props();

  function order(device: PlannedDevice): { text: string; cls: string } {
    if (device.canary) return { text: "first (staged)", cls: "text-warning-300" };
    if (device.plan.concurrency.kind === "exclusive")
      return { text: `one at a time (${device.plan.concurrency.resource})`, cls: "text-secondary-200" };
    return { text: "parallel", cls: "text-surface-300" };
  }
</script>

<ul class="divide-y divide-surface-800 rounded-base border border-surface-800">
  {#each plan.devices as device (keyString(device.device))}
    {@const o = order(device)}
    <li class="grid grid-cols-[1fr_auto] gap-x-3 gap-y-0.5 px-3 py-2 text-xs">
      <span class="truncate font-medium text-surface-50">{device.name}</span>
      <span class="font-mono text-[0.7rem]">
        <span class="text-surface-400">{device.from_version ?? "?"}</span>
        <i class="fa-solid fa-arrow-right mx-1 text-[0.55rem] text-surface-500" aria-hidden="true"></i>
        <span class="text-surface-50">{device.release.version}</span>
      </span>
      <span class="text-[0.65rem] text-surface-400">{device.plan.summary}</span>
      <span class="text-right text-[0.65rem] {o.cls}">{o.text}</span>
    </li>
  {/each}
</ul>
