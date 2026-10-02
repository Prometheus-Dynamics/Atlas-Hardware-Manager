<script lang="ts">
  // Stage cards for one device's job: done (tick), current (red tint,
  // spinner, percent), next (quiet). Failed stages turn rose.
  import type { DeviceJobState } from "$lib/api/client";
  import { stageStates, stepLabel, isRecoveryPlan } from "$lib/present";
  import { pop } from "$lib/ui/motion";
  import Icon from "./Icon.svelte";

  let { job }: { job: DeviceJobState } = $props();

  const stages = $derived(stageStates(job));
  const recovery = $derived(isRecoveryPlan(job.plan));
</script>

<ol class="stages" aria-label="Stages">
  {#each stages as stage, i (stage.step)}
    <li class="stage {stage.state}" aria-current={stage.state === "current" ? "step" : undefined}>
      <span class="badge">
        {#if stage.state === "done"}
          <span class="inline-flex" in:pop><Icon name="check" size={14} stroke={2.5} /></span>
        {:else if stage.state === "current"}
          <Icon name="loader-2" size={14} stroke={2.25} class="spin" />
        {:else if stage.state === "failed"}
          <Icon name="x" size={14} stroke={2.5} />
        {:else}
          <span class="num">{i + 1}</span>
        {/if}
      </span>
      <span class="name">{stepLabel(stage.step, recovery)}</span>
      <span class="meta">
        {#if stage.state === "done"}Done{:else if stage.state === "current"}{Math.round(job.fraction * 100)}%{:else if stage.state === "failed"}Stopped{:else}Next{/if}
      </span>
    </li>
  {/each}
</ol>

<style>
  .stages {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(110px, 1fr));
    gap: 8px;
  }
  .stage {
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 12px;
    border-radius: var(--r-card);
    background: var(--glass);
    border: 0.5px solid var(--glass-border);
    transition:
      background var(--t-med),
      border-color var(--t-med);
  }
  .badge {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 24px;
    height: 24px;
    border-radius: 50%;
    background: var(--glass-strong);
    color: var(--fg-faint);
  }
  .num {
    font-size: 12px;
    font-weight: 600;
  }
  .name {
    font-size: 13px;
    font-weight: 500;
    color: var(--fg-muted);
  }
  .meta {
    font-size: 12px;
    color: var(--fg-faint);
  }
  .done .badge {
    background: var(--ok-bg);
    color: var(--ok-fg);
  }
  .done .name {
    color: var(--fg);
  }
  .current {
    background: var(--accent-tint);
    border-color: var(--accent-tint-strong);
  }
  .current .badge {
    background: var(--accent);
    color: var(--on-accent);
  }
  .current .name {
    color: var(--fg);
  }
  .current .meta {
    color: var(--accent-text-strong);
    font-variant-numeric: tabular-nums;
  }
  .failed {
    background: var(--err-bg);
    border-color: var(--err-bg);
  }
  .failed .badge {
    background: var(--err);
    color: var(--bg);
  }
  .failed .meta {
    color: var(--err-fg);
  }
  .upcoming {
    opacity: 0.75;
  }
</style>
