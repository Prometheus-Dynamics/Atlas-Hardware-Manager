<script lang="ts">
  // One device inside a multi-device job: compact stages and bar, with
  // the full stage cards and log a click away.
  import { slide } from "svelte/transition";
  import type { DeviceJobState } from "#lib/api/client.ts";
  import Icon from "#lib/components/common/Icon.svelte";
  import Pill from "#lib/components/common/Pill.svelte";
  import ProgressBar from "#lib/components/common/ProgressBar.svelte";
  import StageDots from "#lib/components/common/StageDots.svelte";
  import Stepper from "#lib/components/common/Stepper.svelte";
  import { duration, jobStatusDetail, jobStatusTone, overallFraction, sentence } from "#lib/format.ts";
  import { outcomeText, quietFor } from "#lib/present.ts";
  import { clock } from "#lib/stores/clock.svelte.ts";
  import { DUR, ease, ms } from "#lib/ui/motion.ts";
  import LogTail from "./LogTail.svelte";
  import StageProgress from "./StageProgress.svelte";

  let { job }: { job: DeviceJobState } = $props();

  let open = $state(false);
  const tone = $derived(jobStatusTone(job.status));
  const detail = $derived(job.status.status === "verified" ? null : jobStatusDetail(job.status));
  const elapsed = $derived(job.started_ms ? (job.finished_ms ?? clock.now) - job.started_ms : null);
  const quiet = $derived(quietFor(job, clock.now));
  const bar = $derived(tone === "error" ? "error" : tone === "warning" ? "warning" : tone === "success" ? "success" : tone === "neutral" ? "neutral" : "primary");
</script>

<li class="glass item" class:open>
  <button type="button" class="head" aria-expanded={open} onclick={() => (open = !open)}>
    <span class="chev" class:open><Icon name="chevron-right" size={14} /></span>
    <span class="min-w-0">
      <span class="block truncate text-[13px] font-medium text-fg">
        {job.name}{#if job.canary}<span class="ml-2 text-[12px] font-normal text-warn-fg">Goes first</span>{/if}
      </span>
      <span class="mono block truncate text-[11.5px] text-fg-faint">{job.release.version}</span>
    </span>
    <StageDots {job} />
    <span class="w-full"><ProgressBar value={overallFraction(job)} tone={bar} size={5} label="{job.name} progress" /></span>
    <span class="flex items-center justify-end gap-2">
      {#if elapsed !== null}<span class="mono text-[11.5px] text-fg-faint">{duration(elapsed)}</span>{/if}
      {#if quiet !== null}<Pill tone="warning" icon="alert-triangle" label="Quiet {duration(quiet)}" title="No report from the device for {duration(quiet)}" />{/if}
      <Pill {tone} label={job.status.status === "running" ? `${Math.round(overallFraction(job) * 100)}%` : job.status.status === "verified" ? "Verified" : outcomeText(job)} />
    </span>
  </button>
  {#if detail && !open}<p class="px-4 pb-3 pl-11 text-[12.5px] text-fg-muted">{sentence(detail)}</p>{/if}
  {#if open}
    <div class="flex flex-col gap-4 px-4 pb-4 pt-1" transition:slide={{ duration: ms(DUR.med), easing: ease }}>
      <Stepper {job} />
      <StageProgress {job} />
      {#if detail}<p class="text-[13px] text-fg-muted">{sentence(detail)}</p>{/if}
      <LogTail lines={job.log} />
    </div>
  {/if}
</li>

<style>
  .item {
    overflow: hidden;
    transition: background var(--t-fast);
  }
  .item:hover,
  .item.open {
    background: var(--glass-hover);
  }
  .head {
    display: grid;
    grid-template-columns: 16px minmax(120px, 1.2fr) auto minmax(80px, 1fr) minmax(110px, auto);
    align-items: center;
    gap: 14px;
    width: 100%;
    padding: 12px 16px;
    text-align: left;
  }
  .chev {
    display: inline-flex;
    color: var(--fg-faint);
    transition: transform var(--t-med) var(--ease-out);
  }
  .chev.open {
    transform: rotate(90deg);
  }
</style>
