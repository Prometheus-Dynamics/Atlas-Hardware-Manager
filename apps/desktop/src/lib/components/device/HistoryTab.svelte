<script lang="ts">
  import { goto } from "$app/navigation";
  import { keyString, sameKey, type DeviceRecord, type HistoryEntry } from "#lib/api/client.ts";
  import Icon from "#lib/components/common/Icon.svelte";
  import Pill from "#lib/components/common/Pill.svelte";
  import ProgressBar from "#lib/components/common/ProgressBar.svelte";
  import { clockTime, jobStatusDetail, jobStatusLabel, jobStatusTone, overallFraction, sentence } from "#lib/format.ts";
  import { activity } from "#lib/stores/activity.svelte.ts";
  import { jobs } from "#lib/stores/jobs.svelte.ts";
  import { deviceStatus } from "#lib/stores/status.svelte.ts";
  import { ui } from "#lib/stores/ui.svelte.ts";
  import { rise } from "#lib/ui/motion.ts";
  import BootsCard from "./BootsCard.svelte";
  import DeviceTimeline from "./DeviceTimeline.svelte";

  let { record }: { record: DeviceRecord } = $props();

  const entries = $derived(
    jobs.sorted.flatMap((job) =>
      job.devices.filter((d) => sameKey(d.device, record.key)).map((state, i) => ({ job, state, row: `${job.id}:${i}` })),
    ),
  );

  // The merged history (Atlas's record and the board's event log); until it
  // loads, Atlas's own entries for this device.
  $effect(() => deviceStatus.watch(record));
  const merged = $derived(deviceStatus.history.get(keyString(record.key)));
  const timeline = $derived<HistoryEntry[]>(
    merged ??
      activity.entries
        .filter((e) => sameKey(e.device, record.key))
        .map((e) => ({
          at_ms: e.at_ms,
          level: e.level,
          kind: e.kind,
          source: "atlas",
          origin: "atlas",
          message: e.message,
          boot_id: null,
          data: {},
        })),
  );

  // The last 14 days, a bar each: how much happened, and how much of it
  // was a warning or an error.
  const DAYS = 14;
  const days = $derived.by(() => {
    const today = new Date();
    today.setHours(0, 0, 0, 0);
    const start = today.getTime() - (DAYS - 1) * 86_400_000;
    const out = Array.from({ length: DAYS }, (_, i) => ({ at: start + i * 86_400_000, ok: 0, bad: 0 }));
    for (const e of timeline) {
      const i = Math.floor((e.at_ms - start) / 86_400_000);
      if (i < 0 || i >= DAYS) continue;
      if (e.level === "warning" || e.level === "error") out[i].bad += 1;
      else out[i].ok += 1;
    }
    return out;
  });
  const warnings = $derived(days.reduce((n, d) => n + d.bad, 0));
  const peak = $derived(Math.max(1, ...days.map((d) => d.ok + d.bad)));
  const bySource = $derived(
    (["atlas", "orion", "local"] as const).map((s) => ({ source: s, n: timeline.filter((e) => e.source === s).length })).filter((s) => s.n > 0),
  );
  const LABEL = { atlas: "you, through Atlas", orion: "Orion", local: "on the board" } as const;
  const status = $derived(deviceStatus.get(record));

  function open(id: number) {
    ui.selectedJob = id;
    ui.close();
    void goto("/jobs");
  }
</script>

{#if entries.length === 0 && timeline.length === 0}
  <div class="flex flex-col items-center gap-2 py-8 text-center">
    <Icon name="history" size={22} class="text-fg-faint" />
    <p class="text-[13px] text-fg-muted">Nothing has happened to this device yet.</p>
  </div>
{:else if entries.length > 0}
  <ul class="flex flex-col gap-2">
    {#each entries as { job, state, row } (row)}
      {@const detail = jobStatusDetail(state.status)}
      <li in:rise>
        <button type="button" class="glass row w-full px-4 py-3 text-left" onclick={() => open(job.id)}>
          <div class="flex items-center justify-between gap-2">
            <span class="text-[13px] font-medium text-fg">Job #{job.id} · <span class="mono">{state.release.version}</span></span>
            <Pill tone={jobStatusTone(state.status)} label={jobStatusLabel(state.status)} />
          </div>
          <p class="mt-0.5 text-[12px] text-fg-faint">{clockTime(job.created_ms)}</p>
          {#if state.status.status === "running"}
            <div class="mt-2"><ProgressBar value={overallFraction(state)} label="Progress" /></div>
          {/if}
          {#if detail}<p class="mt-1.5 text-[12.5px] text-fg-muted">{sentence(detail)}</p>{/if}
        </button>
      </li>
    {/each}
  </ul>
{/if}

{#if timeline.length > 0}
  <div class="history" class:mt-4={entries.length > 0}>
    <section class="panel min-h-0" aria-label="Timeline">
      <div class="panel-head">
        <h2 class="panel-title">Timeline</h2>
        <span class="panel-note">{timeline.length} events, newest first</span>
      </div>
      <div class="min-h-0 flex-1 overflow-y-auto"><DeviceTimeline entries={timeline} limit={Infinity} /></div>
    </section>
    <div class="side">
      <section class="panel" aria-label="Last 14 days">
        <div class="panel-head">
          <h2 class="panel-title">Last 14 days</h2>
          <span class="panel-note">{days.reduce((n, d) => n + d.ok + d.bad, 0)} events · {warnings} warning{warnings === 1 ? "" : "s"}</span>
        </div>
        <div class="bars" role="img" aria-label="Events per day over the last 14 days">
          {#each days as d (d.at)}
            <span class="day" title="{new Date(d.at).toLocaleDateString()}: {d.ok + d.bad} events, {d.bad} warnings or errors">
              <span class="bad" style:height="{(d.bad / peak) * 100}%"></span>
              <span class="ok" style:height="{(d.ok / peak) * 100}%"></span>
            </span>
          {/each}
        </div>
        <div class="axis"><span>{new Date(days[0].at).toLocaleDateString(undefined, { month: "short", day: "numeric" })}</span><span>today</span></div>
        {#if bySource.length}
          <dl class="sources">
            {#each bySource as s (s.source)}
              <div><dt>{LABEL[s.source]}</dt><dd>{s.n}</dd></div>
            {/each}
          </dl>
        {/if}
      </section>
      <BootsCard {record} {status} />
    </div>
  </div>
{/if}

<style>
  .history {
    flex: 1;
    min-height: 0;
    display: grid;
    gap: 12px;
    grid-template-columns: minmax(0, 1fr);
  }
  @container (min-width: 900px) {
    .history {
      grid-template-columns: minmax(0, 1.6fr) minmax(0, 1fr);
    }
  }
  .side {
    display: flex;
    flex-direction: column;
    gap: 12px;
    min-width: 0;
  }
  .bars {
    display: flex;
    align-items: stretch;
    gap: 3px;
    height: 90px;
  }
  .day {
    flex: 1;
    display: flex;
    flex-direction: column-reverse;
    background: var(--inset);
    border-radius: 1px;
  }
  .day .ok {
    background: var(--series-1);
  }
  .day .bad {
    background: var(--warn);
  }
  .axis {
    display: flex;
    justify-content: space-between;
    font-size: 10.5px;
    color: var(--fg-faint);
  }
  .sources {
    display: flex;
    flex-direction: column;
    border-top: 1px solid var(--hairline);
    padding-top: 6px;
    font-size: 12px;
  }
  .sources div {
    display: flex;
    justify-content: space-between;
    padding: 2px 0;
  }
  .sources dt {
    color: var(--fg-muted);
  }
  .sources dd {
    font-variant-numeric: tabular-nums;
  }
  .row {
    transition:
      background var(--t-fast),
      transform var(--t-fast);
  }
  .row:hover {
    background: var(--glass-hover);
  }
  .row:active {
    transform: scale(0.99);
  }
</style>
