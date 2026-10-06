<script lang="ts">
  // A compact row of stage dots for lists and the jobs tray.
  import type { DeviceJobState } from "#lib/api/client.ts";
  import { stageStates, stepLabel, isRecoveryPlan } from "#lib/present.ts";

  let { job }: { job: DeviceJobState } = $props();
  const stages = $derived(stageStates(job));
  const recovery = $derived(isRecoveryPlan(job.plan));
</script>

<span class="dots" aria-hidden="true">
  {#each stages as stage (stage.step)}
    <span class="d {stage.state}" title="{stepLabel(stage.step, recovery)}: {stage.state}"></span>
  {/each}
</span>

<style>
  .dots {
    display: inline-flex;
    gap: 4px;
    align-items: center;
  }
  .d {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: var(--glass-strong);
    transition:
      background var(--t-med),
      transform var(--t-med) var(--ease-out);
  }
  .done {
    background: var(--ok);
  }
  .current {
    background: var(--accent);
    transform: scale(1.3);
    box-shadow: 0 0 0 3px var(--accent-tint);
  }
  .failed {
    background: var(--err);
  }
</style>
