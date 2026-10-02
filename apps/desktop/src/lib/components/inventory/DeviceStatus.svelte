<script lang="ts">
  // The one status line for a device: progress, waiting, failure, or version.
  import { keyString, type DeviceRecord } from "$lib/api/client";
  import Pill from "$lib/components/common/Pill.svelte";
  import ProgressBar from "$lib/components/common/ProgressBar.svelte";
  import { jobStatusLabel, overallFraction, primaryVersion, timeAgo } from "$lib/format";
  import { isRecovery, isRecoveryPlan, stepLabel } from "$lib/present";
  import { clock } from "$lib/stores/clock.svelte";
  import { insights } from "$lib/stores/insights.svelte";
  import { jobs } from "$lib/stores/jobs.svelte";

  let { record, compact = false }: { record: DeviceRecord; compact?: boolean } = $props();

  const id = $derived(keyString(record.key));
  const active = $derived(jobs.active.get(id));
  const version = $derived(primaryVersion(record.identity));
  const outcome = $derived(insights.failedIds.has(id) ? insights.lastOutcome.get(id) : undefined);
  const pct = $derived(active ? Math.round(overallFraction(active.state) * 100) : 0);
  const verb = $derived(active && isRecoveryPlan(active.state.plan) ? "Flashing" : "Updating");
</script>

{#if active}
  {#if active.state.status.status === "queued"}
    <Pill tone="neutral" icon="clock" label="Queued" />
  {:else}
    <div class="flex w-full min-w-0 flex-col gap-1.5">
      <div class="flex items-center justify-between gap-2 text-[12.5px]">
        <span class="font-medium text-accent-text">{verb} · {pct}%</span>
        {#if !compact && active.state.step}
          <span class="truncate text-fg-faint">{stepLabel(active.state.step, isRecoveryPlan(active.state.plan))}</span>
        {/if}
      </div>
      <ProgressBar value={overallFraction(active.state)} label="{verb} progress" size={compact ? 4 : 6} />
    </div>
  {/if}
{:else if record.presence !== "online"}
  <Pill tone="neutral" icon="plug-connected-x" label="Offline · {timeAgo(record.last_seen_ms, clock.now)}" />
{:else if isRecovery(record)}
  <Pill tone="primary" icon="usb" label={compact ? "USB boot" : "Waiting in USB boot"} />
{:else if outcome}
  <Pill tone="error" icon="alert-circle" label={jobStatusLabel(outcome.state.status)} />
{:else if insights.outdatedIds.has(id)}
  <span class="flex min-w-0 items-center gap-2">
    <Pill tone="warning" icon="arrow-up" label={compact ? `${insights.target(record)} available` : `Update to ${insights.target(record)}`} />
    {#if !compact && version}<span class="mono truncate text-[12px] text-fg-faint">{version}</span>{/if}
  </span>
{:else if version && insights.target(record)}
  <span class="flex min-w-0 items-center gap-2">
    <Pill tone="success" icon="check" label="Up to date" />
    {#if !compact}<span class="mono truncate text-[12px] text-fg-faint">{version}</span>{/if}
  </span>
{:else if version}
  <Pill tone="neutral" mono label={version} />
{/if}
