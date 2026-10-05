<script lang="ts">
  import { keyString, type JobRecord } from "$lib/api/client";
  import Button from "$lib/components/common/Button.svelte";
  import IconTile from "$lib/components/common/IconTile.svelte";
  import Pill from "$lib/components/common/Pill.svelte";
  import Skeleton from "$lib/components/common/Skeleton.svelte";
  import Stepper from "$lib/components/common/Stepper.svelte";
  import { duration, jobStatusDetail, jobStatusTone, sentence, timeAgo } from "$lib/format";
  import { isRecoveryPlan, jobTitle, outcomeText } from "$lib/present";
  import { clock } from "$lib/stores/clock.svelte";
  import { jobs } from "$lib/stores/jobs.svelte";
  import { rise } from "$lib/ui/motion";
  import JobDeviceRow from "./JobDeviceRow.svelte";
  import LogTail from "./LogTail.svelte";
  import StageProgress from "./StageProgress.svelte";

  let { job }: { job: JobRecord } = $props();

  const running = $derived(job.state === "running");
  const single = $derived(job.devices.length === 1 ? job.devices[0] : null);
  const problems = $derived(job.summary ? job.summary.failed + job.summary.rolled_back + job.summary.needs_recovery : 0);
  const image = $derived.by(() => {
    const names = [...new Set(job.devices.map((d) => d.release.artifact?.name ?? d.release.version))];
    return names.length === 1 ? names[0] : `${names.length} releases`;
  });
  const total = $derived((job.finished_ms ?? clock.now) - job.created_ms);
  const tile = $derived(
    running ? ("accent" as const) : problems > 0 ? ("err" as const) : job.state === "cancelled" ? ("muted" as const) : ("ok" as const),
  );
  const icon = $derived(single && isRecoveryPlan(single.plan) ? ("bolt" as const) : ("arrow-up" as const));
</script>

<section class="glass detail flex flex-col gap-6 p-6">
  <header class="flex items-start gap-4">
    <IconTile icon={running ? icon : problems > 0 ? "alert-circle" : job.state === "cancelled" ? "player-stop" : "circle-check"} tone={tile} size={44} />
    <div class="min-w-0 flex-1">
      <h2 class="truncate text-[20px] font-semibold tracking-[-0.01em] text-fg">{jobTitle(job)}</h2>
      <p class="mt-0.5 truncate text-[13px] text-fg-muted">
        <span class="mono">{image}</span> · started {timeAgo(job.created_ms, clock.now)}{running ? "" : ` · took ${duration(total)}`}
      </p>
    </div>
    {#if running}
      <Button icon="player-stop" action={() => jobs.cancel(job.id)}>Cancel</Button>
    {:else if job.summary}
      <Pill
        tone={problems > 0 ? "error" : job.state === "cancelled" ? "neutral" : "success"}
        icon={problems > 0 ? "alert-circle" : job.state === "cancelled" ? "player-stop" : "circle-check"}
        label={problems > 0 ? `${problems} didn't finish` : job.state === "cancelled" ? "Cancelled" : "All verified"}
      />
    {/if}
  </header>

  {#if job.devices.length === 0}
    <div class="flex flex-col gap-3"><Skeleton height={80} /><Skeleton height={40} /></div>
  {:else if single}
    <Stepper job={single} />
    <StageProgress job={single} />
    {#if !running || single.status.status !== "running"}
      {@const detail = jobStatusDetail(single.status)}
      <div class="flex items-center gap-3" in:rise>
        <Pill tone={jobStatusTone(single.status)} label={outcomeText(single)} />
        {#if detail && single.status.status !== "verified"}<span class="text-[13px] text-fg-muted">{sentence(detail)}</span>{/if}
      </div>
    {/if}
    {#if isRecoveryPlan(single.plan) && single.status.status === "verified"}
      {@const notEjected = single.log.some((line) => line.startsWith("could not eject"))}
      <div class="next" in:rise>
        <IconTile icon="power" tone="accent" size={36} />
        <div class="min-w-0 flex-1">
          <p class="text-[14px] font-semibold text-fg">Next: power-cycle {single.name}</p>
          <p class="mt-0.5 text-[13px] leading-relaxed text-fg-muted">
            {#if notEjected}Atlas couldn't eject the eMMC, so unmount it from your file manager first. Then unplug{:else}The eMMC is ejected and safe to unplug. Unplug{/if}
            the board's power and plug it back in <strong class="text-fg">without</strong> holding the boot button. The first start can take a minute
            or two.
          </p>
        </div>
      </div>
    {/if}
    <LogTail lines={single.log} />
  {:else}
    <ul class="flex flex-col gap-2">
      {#each job.devices as state (keyString(state.device))}
        <JobDeviceRow job={state} />
      {/each}
    </ul>
  {/if}
</section>

<style>
  .next {
    display: flex;
    gap: 14px;
    align-items: flex-start;
    padding: 14px 16px;
    border-radius: var(--r-card);
    border: 1px solid color-mix(in srgb, var(--accent) 35%, transparent);
    background: var(--accent-tint);
  }
  .detail {
    border-radius: var(--r-panel);
  }
</style>
