<script lang="ts">
  // The current stage of one device: name, percent, a big bar, and an ETA
  // derived from how fast this stage has moved so far.
  import type { DeviceJobState } from "$lib/api/client";
  import ProgressBar from "$lib/components/common/ProgressBar.svelte";
  import { isRecoveryPlan, stepLabel } from "$lib/present";
  import { clock } from "$lib/stores/clock.svelte";

  let { job }: { job: DeviceJobState } = $props();

  let started = $state<{ step: string | null; at: number }>({ step: null, at: Date.now() });
  $effect(() => {
    if (job.step !== started.step) started = { step: job.step, at: Date.now() };
  });

  const eta = $derived.by(() => {
    const elapsed = clock.now - started.at;
    const f = job.fraction;
    if (f < 0.08 || f >= 1 || elapsed < 1500) return null;
    const left = Math.round(((elapsed / f) * (1 - f)) / 1000);
    if (left < 5) return "a few seconds left";
    if (left < 60) return `about ${left} s left`;
    return `about ${Math.round(left / 60)} min left`;
  });
  const recovery = $derived(isRecoveryPlan(job.plan));
</script>

{#if job.step && job.status.status === "running"}
  <div class="flex flex-col gap-2.5">
    <div class="flex items-baseline justify-between gap-3">
      <span class="text-[14px] font-semibold text-fg">
        {stepLabel(job.step, recovery)}
        <span class="ml-1 font-medium tabular-nums text-accent-text">{Math.round(job.fraction * 100)}%</span>
      </span>
      {#if eta}<span class="text-[12.5px] text-fg-faint">{eta}</span>{/if}
    </div>
    <ProgressBar value={job.fraction} size={8} label="{stepLabel(job.step, recovery)} progress" />
  </div>
{/if}
