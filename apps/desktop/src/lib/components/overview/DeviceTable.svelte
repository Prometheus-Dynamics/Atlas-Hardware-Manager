<script lang="ts">
  // The robot's devices as rows: state, version (and the one it should be
  // on), a CPU trend, temperature and vision rate as bars. A row opens the
  // device.
  import { keyString, type DeviceRecord } from "#lib/api/client.ts";
  import Sparkline from "#lib/components/common/Sparkline.svelte";
  import { deviceName, primaryVersion } from "#lib/format.ts";
  import { metricTone } from "#lib/metrics.ts";
  import { displayModel, isRecovery } from "#lib/present.ts";
  import { insights } from "#lib/stores/insights.svelte.ts";
  import { live } from "#lib/stores/live.svelte.ts";
  import { devicePath } from "#lib/stores/ui.svelte.ts";

  let { records }: { records: DeviceRecord[] } = $props();

  const rows = $derived(
    [...records]
      .sort((a, b) => order(a) - order(b) || deviceName(a).localeCompare(deviceName(b), undefined, { numeric: true }))
      .map((r) => {
        const id = keyString(r.key);
        const temp = live.metric(id, "temp");
        const fps = live.metric(id, "fps");
        const cpu = live.metric(id, "cpu");
        const target = insights.outdatedIds.has(id) ? insights.target(r) : null;
        return { record: r, id, temp, fps, cpu, cpuSeries: live.series(id, "cpu"), target, version: primaryVersion(r.identity) };
      }),
  );

  function order(r: DeviceRecord): number {
    if (r.presence !== "online") return 3;
    if (isRecovery(r)) return 0;
    return insights.outdatedIds.has(keyString(r.key)) ? 1 : 2;
  }

  function dot(r: DeviceRecord): string {
    if (r.presence !== "online") return "var(--offline)";
    if (isRecovery(r)) return "var(--accent)";
    return "var(--ok)";
  }

  const pct = (v: number, lo: number, hi: number) => `${Math.max(0, Math.min(100, ((v - lo) / (hi - lo)) * 100))}%`;
</script>

<section class="rows" aria-label="Devices">
  <table>
    <thead>
      <tr>
        <th scope="col">Device</th>
        <th scope="col">Version</th>
        <th scope="col">CPU</th>
        <th scope="col">Temperature</th>
        <th scope="col">Vision</th>
      </tr>
    </thead>
    <tbody>
      {#each rows as row (row.id)}
        <tr>
          <td>
            <a href={devicePath(row.id)} class="name">
              <span class="dot" style:background={dot(row.record)}></span>
              <span class="font-medium">{deviceName(row.record)}</span>
              <span class="muted truncate">{displayModel(row.record)}</span>
            </a>
          </td>
          <td class="nowrap">
            <span class="mono">{row.version ?? (isRecovery(row.record) ? "USB boot" : "—")}</span>
            {#if row.target}<span class="behind">→ {row.target}</span>{/if}
          </td>
          <td class="nowrap">
            {#if row.cpu}
              <span class="inline-flex items-center gap-2">
                <span class="spark"><Sparkline values={row.cpuSeries} color="var(--series-1)" height={18} min={0} max={100} /></span>
                <span class="num">{Math.round(row.cpu.value)} %</span>
              </span>
            {:else}<span class="muted">—</span>{/if}
          </td>
          <td class="nowrap">
            {#if row.temp}
              {@const hot = metricTone(row.temp) === "warn" || metricTone(row.temp) === "err"}
              <span class="inline-flex items-center gap-2">
                <span class="bar"><span style:width={pct(row.temp.value, 30, row.temp.max ?? 90)} style:background={hot ? "var(--warn)" : "var(--series-1)"}></span></span>
                <span class="num" class:hot>{row.temp.value.toFixed(1)} °C</span>
              </span>
            {:else}<span class="muted">—</span>{/if}
          </td>
          <td class="nowrap">
            {#if row.fps}
              <span class="inline-flex items-center gap-2">
                <span class="bar"><span style:width={pct(row.fps.value, 0, row.fps.max ?? 60)} style:background="var(--series-2)"></span></span>
                <span class="num">{Math.round(row.fps.value)} fps</span>
              </span>
            {:else}<span class="muted">—</span>{/if}
          </td>
        </tr>
      {/each}
    </tbody>
  </table>
</section>

<style>
  .rows {
    min-height: 0;
    overflow: auto;
    border-radius: var(--r-card);
    background: var(--glass);
    border: 1px solid var(--glass-border);
  }
  table {
    width: 100%;
    border-collapse: collapse;
    font-size: 12.5px;
  }
  th {
    position: sticky;
    top: 0;
    background: var(--glass);
    text-align: left;
    font-size: 11px;
    font-weight: 500;
    color: var(--fg-faint);
    padding: 9px 8px;
  }
  th:first-child,
  td:first-child {
    padding-left: 14px;
  }
  td {
    padding: 6px 8px;
    border-top: 1px solid var(--hairline);
  }
  tr:hover td {
    background: var(--glass-hover);
  }
  .name {
    display: flex;
    align-items: center;
    gap: 9px;
    min-width: 0;
    color: var(--fg);
  }
  .dot {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    flex-shrink: 0;
  }
  .muted {
    font-size: 11.5px;
    color: var(--fg-faint);
  }
  .mono {
    font-family: var(--font-code);
    font-size: 12px;
  }
  .behind {
    margin-left: 6px;
    font-size: 11px;
    color: var(--warn-fg);
  }
  .nowrap {
    white-space: nowrap;
  }
  .spark {
    display: inline-block;
    width: 90px;
  }
  .bar {
    display: inline-block;
    width: 54px;
    height: 6px;
    border-radius: 1px;
    background: var(--glass-strong);
    overflow: hidden;
  }
  .bar span {
    display: block;
    height: 100%;
  }
  .num {
    font-variant-numeric: tabular-nums;
    min-width: 46px;
  }
  .num.hot {
    color: var(--warn-fg);
  }
</style>
