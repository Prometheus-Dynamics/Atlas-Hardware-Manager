<script lang="ts">
  import { keyString, type DeviceRecord } from "$lib/api/client";
  import Checkbox from "$lib/components/common/Checkbox.svelte";
  import IconTile from "$lib/components/common/IconTile.svelte";
  import StatusDot, { type DotState } from "$lib/components/common/StatusDot.svelte";
  import { deviceName } from "$lib/format";
  import { metricTone, metricValue } from "$lib/metrics";
  import { deviceIcon, deviceSubline, isRecovery, viaName } from "$lib/present";
  import { devices } from "$lib/stores/devices.svelte";
  import { insights } from "$lib/stores/insights.svelte";
  import { jobs } from "$lib/stores/jobs.svelte";
  import { live } from "$lib/stores/live.svelte";
  import { ui } from "$lib/stores/ui.svelte";
  import DeviceStatus from "./DeviceStatus.svelte";
  import { clickCheck, clickDevice } from "./select";

  let { record }: { record: DeviceRecord } = $props();

  const id = $derived(keyString(record.key));
  const name = $derived(deviceName(record));
  const selected = $derived(ui.selection.has(id));
  const focused = $derived(ui.focused === id);
  const open = $derived(ui.panel?.kind === "device" && ui.panel.key === id);
  const online = $derived(record.presence === "online");
  const waiting = $derived(online && isRecovery(record));
  const via = $derived(viaName(record, devices.nameOf));
  const fresh = $derived(devices.fresh.has(id));

  /** Up to two live readings, when the device reports them. */
  const glance = $derived(
    online
      ? ["temp", "fps", "voltage", "cpu"]
          .map((m) => live.metric(id, m))
          .filter((m) => !!m)
          .slice(0, 2)
      : [],
  );

  const dot = $derived.by((): { state: DotState; label: string } => {
    if (!online) return { state: "offline", label: "Offline" };
    if (jobs.active.has(id)) return { state: "busy", label: "Updating" };
    if (waiting) return { state: "busy", label: "Waiting in USB boot" };
    if (insights.failedIds.has(id)) return { state: "failed", label: "Last update failed" };
    return { state: "online", label: "Online" };
  });

  let card: HTMLElement | undefined = $state();
  $effect(() => {
    if (focused) card?.scrollIntoView({ block: "nearest", behavior: "smooth" });
  });
</script>

<article
  bind:this={card}
  class="card"
  class:selected
  class:focused
  class:open
  class:offline={!online}
  class:waiting
  class:fresh
>
  <button type="button" class="hit" aria-label="Open {name}" onclick={(e) => clickDevice(e, id)}></button>

  <div class="check" class:show={selected || ui.selection.size > 0}>
    <Checkbox checked={selected} label="Select {name}" onclick={(e) => clickCheck(e, id)} />
  </div>

  <div class="flex items-start justify-between gap-3">
    <IconTile icon={deviceIcon(record)} tone={waiting ? "accent" : online ? "neutral" : "muted"} />
    <StatusDot state={dot.state} label={dot.label} />
  </div>

  <div class="mt-3 min-w-0">
    <h3 class="truncate text-[14px] font-semibold text-fg" title={name}>{name}</h3>
    <p class="truncate text-[12.5px] text-fg-muted">{deviceSubline(record)}</p>
    {#if via}<p class="truncate text-[12px] text-fg-faint">via {via}</p>{/if}
  </div>

  {#if glance.length > 0}
    <div class="glance">
      {#each glance as metric (metric.id)}
        {@const shown = metricValue(metric)}
        <span class="reading {metricTone(metric)}">{shown.value}<small>{shown.unit}</small></span>
      {/each}
    </div>
  {/if}

  <div class="mt-auto flex min-h-[26px] items-end pt-3">
    <DeviceStatus {record} />
  </div>
</article>

<style>
  .glance {
    display: flex;
    gap: 10px;
    margin-top: 8px;
    font-size: 12px;
    font-weight: 500;
    color: var(--fg-muted);
    font-variant-numeric: tabular-nums;
  }
  .reading small {
    margin-left: 2px;
    font-size: 11px;
    color: var(--fg-faint);
  }
  .reading.warn {
    color: var(--warn-fg);
  }
  .reading.err {
    color: var(--err-fg);
  }
  .card {
    position: relative;
    display: flex;
    flex-direction: column;
    min-height: 168px;
    padding: 16px;
    border-radius: var(--r-card);
    background: var(--glass);
    border: 1px solid var(--glass-border);
    transition:
      transform var(--t-med) var(--ease-out),
      background var(--t-fast),
      border-color var(--t-fast),
      box-shadow var(--t-med);
  }
  .card > :not(.hit, .check) {
    pointer-events: none;
    position: relative;
  }
  .card:hover {
    background: var(--glass-hover);
    transform: translateY(-2px);
    box-shadow: var(--shadow-lift);
  }
  .card:active {
    transform: translateY(0) scale(0.99);
  }
  .card.open {
    border-color: var(--glass-border-strong);
    background: var(--glass-hover);
  }
  .card.focused {
    box-shadow: 0 0 0 1.5px var(--glass-border-strong);
  }
  .card.selected {
    border-color: var(--accent-ring);
    box-shadow:
      0 0 0 1px var(--accent-ring),
      0 8px 24px -10px var(--accent-glow);
  }
  .card.waiting {
    background: linear-gradient(160deg, var(--accent-tint), var(--glass) 60%);
    border-color: var(--accent-tint-strong);
  }
  .card.offline {
    opacity: 0.62;
  }
  .card.offline:hover {
    opacity: 0.85;
  }
  .card.fresh {
    animation: arrive 1.6s var(--ease-out);
  }
  @keyframes arrive {
    from {
      box-shadow: 0 0 0 1.5px var(--ok);
    }
  }
  .hit {
    position: absolute;
    inset: 0;
    border-radius: inherit;
    z-index: 0;
    cursor: pointer;
  }
  .hit:focus-visible {
    outline-offset: 2px;
  }
  .check {
    position: absolute;
    top: -7px;
    left: -7px;
    z-index: 2;
    opacity: 0;
    transform: scale(0.8);
    transition:
      opacity var(--t-fast),
      transform var(--t-fast) var(--ease-out);
  }
  .check :global(.check) {
    box-shadow: 0 0 0 3px var(--bg);
  }
  .card:hover .check,
  .check.show,
  .check:focus-within {
    opacity: 1;
    transform: scale(1);
  }
</style>
