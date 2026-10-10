<script lang="ts">
  // One device, compact, beside others on the Monitor page: its status, every live reading as a row with a trend,
  // and its board's streaming sensors as charts.
  import { keyString, type DeviceRecord } from "#lib/api/client.ts";
  import Button from "#lib/components/common/Button.svelte";
  import IconTile from "#lib/components/common/IconTile.svelte";
  import Sparkline from "#lib/components/common/Sparkline.svelte";
  import { deviceName, timeAgo } from "#lib/format.ts";
  import { coreMetrics, isCore, metricValue, sortMetrics } from "#lib/metrics.ts";
  import { deviceIcon, isRecovery, modelName } from "#lib/present.ts";
  import { clock } from "#lib/stores/clock.svelte.ts";
  import { live, watchLive } from "#lib/stores/live.svelte.ts";
  import { deviceStatus } from "#lib/stores/status.svelte.ts";
  import { ui } from "#lib/stores/ui.svelte.ts";
  import HardwareTab from "./HardwareTab.svelte";

  let {
    record,
    onremove,
    removeLabel = "Remove",
  }: {
    record: DeviceRecord;
    /** Takes it off whatever shows it (unpin, off the Monitor page). */
    onremove?: () => void;
    removeLabel?: string;
  } = $props();

  const id = $derived(keyString(record.key));
  const online = $derived(record.presence === "online");
  const metrics = $derived(sortMetrics((live.metrics.get(id) ?? []).filter((m) => !isCore(m))));
  const cores = $derived(coreMetrics(live.metrics.get(id) ?? []));
  const hardware = $derived.by(() => {
    const snap = deviceStatus.byDevice.get(id)?.hardware ?? null;
    return online && snap && snap.devices.length > 0 ? snap : null;
  });

  $effect(() => watchLive([record]));
  $effect(() => deviceStatus.watch(record));

  const tone = (m: (typeof metrics)[number]) =>
    m.warn_above !== null && m.value > m.warn_above ? "var(--warn)" : "var(--info)";
</script>

<section class="monitor">
  <header class="head">
    <IconTile icon={deviceIcon(record)} size={26} tone={isRecovery(record) && online ? "accent" : "neutral"} />
    <div class="min-w-0 flex-1">
      <button type="button" class="name" onclick={() => ui.openDevice(id)} title="Open {deviceName(record)}">{deviceName(record)}</button>
      <p class="sub">
        <span class="dot" class:on={online}></span>{online ? modelName(record) : `Offline · seen ${timeAgo(record.last_seen_ms, clock.now)}`}
      </p>
    </div>
    {#if onremove}<Button size="sm" variant="ghost" icon="x" label="{removeLabel} {deviceName(record)}" onclick={onremove} />{/if}
  </header>

  {#if !online}
    <p class="empty">No live data while it's offline.</p>
  {:else if metrics.length === 0 && !hardware}
    <p class="empty">{live.errors.get(id) ?? "Waiting for readings…"}</p>
  {:else}
    {#if metrics.length}
      <ul class="rows">
        {#each metrics as metric (metric.id)}
          {@const shown = metricValue(metric)}
          <li class="row" title={metric.detail ?? undefined}>
            <span class="label">{metric.label}</span>
            <span class="value">{shown.value}<span class="unit">{shown.unit}</span></span>
            <span class="trend">
              <Sparkline
                values={live.series(id, metric.id)}
                times={live.seriesTimes(id)}
                height={18}
                color={tone(metric)}
                format={(v) => `${v.toLocaleString(undefined, { maximumFractionDigits: 2 })} ${metric.unit ?? ""}`}
              />
            </span>
          </li>
          {#if metric.id === "cpu" && cores.length}
            <li class="cores" aria-label="CPU cores">
              {#each cores as core (core.id)}
                <span class="core" title="{core.label}: {Math.round(core.value)}%"><span style="height: {Math.min(100, core.value)}%"></span></span>
              {/each}
            </li>
          {/if}
        {/each}
      </ul>
    {/if}
    {#if hardware}
      <div class="hw">
        <HardwareTab {hardware} boardKey={record.key} compact />
      </div>
    {/if}
  {/if}
</section>

<style>
  .monitor {
    display: flex;
    flex-direction: column;
    min-width: 0;
    background: var(--glass);
    border: 1px solid var(--glass-border);
    border-radius: var(--r-card);
  }
  .head {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 7px 8px 7px 10px;
    border-bottom: 1px solid var(--hairline);
  }
  .name {
    display: block;
    max-width: 100%;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: 13px;
    font-weight: 600;
    color: var(--fg);
    text-align: left;
  }
  .name:hover {
    text-decoration: underline;
  }
  .sub {
    display: flex;
    align-items: center;
    gap: 5px;
    font-size: 11.5px;
    color: var(--fg-faint);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .dot {
    width: 6px;
    height: 6px;
    border-radius: var(--r-round);
    background: var(--offline);
    flex-shrink: 0;
  }
  .dot.on {
    background: var(--ok);
  }
  .empty {
    padding: 10px;
    font-size: 12.5px;
    color: var(--fg-muted);
  }
  .rows {
    display: flex;
    flex-direction: column;
    padding: 4px 0;
  }
  .row {
    display: grid;
    grid-template-columns: minmax(0, 1fr) auto 72px;
    align-items: center;
    gap: 10px;
    padding: 3px 10px;
    font-size: 12.5px;
  }
  .row:hover {
    background: var(--glass-hover);
  }
  .label {
    color: var(--fg-muted);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .value {
    font-weight: 600;
    color: var(--fg);
    font-variant-numeric: tabular-nums;
    text-align: right;
  }
  .unit {
    margin-left: 2px;
    font-weight: 400;
    color: var(--fg-faint);
    font-size: 11px;
  }
  .cores {
    display: flex;
    gap: 2px;
    height: 16px;
    padding: 0 10px 3px;
  }
  .core {
    flex: 1;
    display: flex;
    align-items: flex-end;
    background: var(--inset);
    border-radius: 1px;
  }
  .core span {
    display: block;
    width: 100%;
    background: var(--info);
    border-radius: 1px;
  }
  .hw {
    padding: 8px;
    border-top: 1px solid var(--hairline);
  }
</style>
