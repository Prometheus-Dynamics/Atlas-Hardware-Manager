<script lang="ts">
  import type { DeviceJobStatus, UpdateStep } from "$lib/api/client";
  import { STEP_LABELS } from "$lib/format";

  let {
    steps,
    current,
    status,
  }: { steps: UpdateStep[]; current: UpdateStep | null; status: DeviceJobStatus } = $props();

  const index = $derived(current ? steps.indexOf(current) : -1);
  const done = $derived(status.status === "verified");
  const failed = $derived(["failed", "rolled-back", "needs-recovery"].includes(status.status));

  function state(i: number): "done" | "active" | "failed" | "todo" {
    if (done || i < index) return "done";
    if (i === index) return failed ? "failed" : status.status === "running" ? "active" : "todo";
    return "todo";
  }

  const cls = {
    done: "border-success-600 text-success-400",
    active: "border-secondary-400 text-secondary-200 bg-secondary-500/15",
    failed: "border-error-500 text-error-300 bg-error-500/15",
    todo: "border-surface-700 text-surface-500",
  };
</script>

<ol class="flex items-center gap-1" aria-label="Update steps">
  {#each steps as step, i (step)}
    {@const s = state(i)}
    <li
      class="rounded-base border px-1.5 text-[0.6rem] uppercase tracking-[0.12em] {cls[s]}"
      aria-current={s === "active" ? "step" : undefined}
    >
      {#if s === "done"}<i class="fa-solid fa-check mr-0.5 text-[0.5rem]" aria-hidden="true"></i>{/if}{STEP_LABELS[step]}
    </li>
  {/each}
</ol>
