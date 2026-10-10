<script lang="ts">
  import { keyString, type DeviceRecord } from "#lib/api/client.ts";
  import Checkbox from "#lib/components/common/Checkbox.svelte";
  import IconTile from "#lib/components/common/IconTile.svelte";
  import StatusDot, { type DotState } from "#lib/components/common/StatusDot.svelte";
  import { deviceName, linkText } from "#lib/format.ts";
  import Icon from "#lib/components/common/Icon.svelte";
  import Sparkline from "#lib/components/common/Sparkline.svelte";
  import { metricFraction, metricIcon, metricTone, metricValue } from "#lib/metrics.ts";
  import type { IconName } from "#lib/ui/icons.ts";
  import { deviceIcon, deviceSubline, isRecovery, viaName } from "#lib/present.ts";
  import { devices } from "#lib/stores/devices.svelte.ts";
  import { insights } from "#lib/stores/insights.svelte.ts";
  import { jobs } from "#lib/stores/jobs.svelte.ts";
  import { live } from "#lib/stores/live.svelte.ts";
  import { ui } from "#lib/stores/ui.svelte.ts";
  import DeviceStatus from "./DeviceStatus.svelte";
  import { clickCheck, clickDevice } from "./select";

  let { record }: { record: DeviceRecord } = $props();

  const id = $derived(keyString(record.key));
  const name = $derived(deviceName(record));
  const selected = $derived(ui.selection.has(id));
  const focused = $derived(ui.focused === id);
  const open = $derived(ui.viewing === id);
  const online = $derived(record.presence === "online");
  const waiting = $derived(online && isRecovery(record));
  const via = $derived(viaName(record, devices.nameOf));
  const fresh = $derived(devices.fresh.has(id));

  /** Up to four live readings, each with a meter and its recent trend. */
  const glance = $derived(
    online
      ? ["temp", "cpu", "fps", "fan", "voltage", "memory"]
          .map((m) => live.metric(id, m))
          .filter((m) => !!m)
          .slice(0, 4)
      : [],
  );
  const LINK_ICON: Record<string, IconName> = {
    "usb-network": "usb",
    "usb-serial": "usb",
    "usb-boot": "usb",
    ethernet: "router",
    simulated: "flask",
    gateway: "sitemap",
  };
  const TONE_COLOR = { ok: "var(--ok)", warn: "var(--warn)", err: "var(--err)", neutral: "var(--info)" } as const;

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

  <!-- Every slot is kept, filled or not, so cards side by side line up. -->
  <div class="top">
    <IconTile icon={deviceIcon(record)} size={32} tone={waiting ? "accent" : online ? "neutral" : "muted"} />
    <div class="min-w-0 flex-1">
      <h3 class="truncate text-[13.5px] font-semibold leading-tight text-fg" title={name}>{name}</h3>
      <p class="truncate text-[11.5px] text-fg-muted">{deviceSubline(record)}</p>
    </div>
    <div class="flex shrink-0 flex-col items-end gap-1">
      <StatusDot state={dot.state} label={dot.label} />
      <span class="link" title={linkText(record.link_kind, devices.nameOf)}>
        <Icon name={LINK_ICON[record.link_kind.kind] ?? "plug-connected"} size={12} />{via ? via : linkText(record.link_kind, devices.nameOf)}
      </span>
    </div>
  </div>

  <div class="glance" class:empty={glance.length === 0}>
    {#each glance as metric (metric.id)}
      {@const shown = metricValue(metric)}
      {@const tone = metricTone(metric)}
      {@const fraction = metricFraction(metric)}
      <div class="reading {tone}" title="{metric.label}: {shown.value} {shown.unit}">
        <span class="r-label"><Icon name={metricIcon(metric)} size={11} />{metric.label}</span>
        <span class="r-value">{shown.value}<small>{shown.unit}</small></span>
        {#if fraction !== null}
          <span class="meter"><span style="width: {fraction * 100}%; background: {TONE_COLOR[tone]}"></span></span>
        {:else}
          <span class="trend"><Sparkline values={live.series(id, metric.id)} height={10} color={TONE_COLOR[tone]} /></span>
        {/if}
      </div>
    {:else}
      <span class="none">{online ? (waiting ? "In USB boot: no readings" : "No live readings") : "Offline"}</span>
    {/each}
  </div>

  <div class="mt-auto flex min-h-[24px] items-end pt-2">
    <DeviceStatus {record} />
  </div>
</article>

<style>
  .top {
    display: flex;
    align-items: flex-start;
    gap: 10px;
  }
  .link {
    display: inline-flex;
    align-items: center;
    gap: 3px;
    max-width: 9rem;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: 10.5px;
    color: var(--fg-faint);
  }
  /* The readings: a 2 × 2 grid of label, value and a meter (or trend). */
  .glance {
    display: grid;
    grid-template-columns: repeat(2, minmax(0, 1fr));
    gap: 6px;
    min-height: 72px;
    margin-top: 10px;
    align-content: start;
  }
  .glance.empty {
    display: flex;
    align-items: center;
  }
  .reading {
    display: flex;
    flex-direction: column;
    gap: 1px;
    min-width: 0;
    padding: 5px 7px 6px;
    background: var(--inset);
    border-radius: var(--r-sm);
    font-variant-numeric: tabular-nums;
  }
  .r-label {
    display: flex;
    align-items: center;
    gap: 4px;
    font-size: 10.5px;
    color: var(--fg-faint);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .r-value {
    font-size: 14px;
    font-weight: 600;
    color: var(--fg);
    line-height: 1.15;
  }
  .r-value small {
    margin-left: 2px;
    font-size: 10.5px;
    font-weight: 400;
    color: var(--fg-faint);
  }
  .reading.warn .r-value {
    color: var(--warn-fg);
  }
  .reading.err .r-value {
    color: var(--err-fg);
  }
  .meter {
    display: block;
    height: 3px;
    margin-top: 3px;
    background: color-mix(in srgb, var(--fg) 8%, transparent);
    border-radius: 1px;
    overflow: hidden;
  }
  .meter span {
    display: block;
    height: 100%;
    border-radius: 1px;
  }
  .trend {
    display: block;
    margin-top: 1px;
  }
  .none {
    font-size: 12px;
    color: var(--fg-faint);
  }
  .card {
    position: relative;
    display: flex;
    flex-direction: column;
    min-height: var(--card-h, 150px);
    padding: 12px;
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
  /* Grounded: hover lightens it and firms the edge; nothing lifts. */
  .card:hover {
    background: var(--glass-hover);
    border-color: var(--glass-border-strong);
  }
  .card:active {
    background: var(--glass-strong);
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
    box-shadow: 0 0 0 1px var(--accent-ring);
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
