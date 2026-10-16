<script lang="ts">
  // The IMU on the Overview, always there: the board in 3D as it is now
  // (the board's fusion when it streams, else a filter here), roll, pitch
  // and heading, the accelerometer and gyro on each axis, and how well it's
  // calibrated. Calibrating is in Settings (PIT).
  import { api, type CalibrationStatus, type DeviceKey } from "#lib/api/client.ts";
  import { ui } from "#lib/stores/ui.svelte.ts";
  import ImuView from "./ImuView.svelte";
  import type { LiveSeries } from "./live.ts";

  let {
    imuId,
    series,
    fused = null,
    light = null,
    tick = 0,
    deviceKey = null,
    oncalibrate,
  }: {
    imuId: string;
    series: LiveSeries | null;
    fused?: LiveSeries | null;
    light?: LiveSeries | null;
    tick?: number;
    deviceKey?: DeviceKey | null;
    oncalibrate?: () => void;
  } = $props();

  const AXES = ["x", "y", "z"];
  const latest = (prefix: string) => {
    void tick;
    if (!series?.count) return AXES.map(() => null);
    const at = series.index(series.count - 1);
    return AXES.map((axis) => {
      const c = series.channels.findIndex((ch) => ch.name === `${prefix}.${axis}`);
      const v = c >= 0 ? series.v[c][at] : NaN;
      return Number.isFinite(v) ? v : null;
    });
  };
  const accel = $derived(latest("acceleration").map((v) => (v === null ? null : v / 9.80665)));
  const gyro = $derived(latest("angular_rate").map((v) => (v === null ? null : (v * 180) / Math.PI)));
  const fmt = (v: number | null, d: number) => (v === null ? "–" : `${v >= 0 ? " " : ""}${v.toFixed(d)}`);

  let calibration = $state<CalibrationStatus | null>(null);
  $effect(() => {
    const key = deviceKey;
    const id = imuId;
    if (!key) return;
    let stale = false;
    api
      .calibrationStatus(key, id)
      .then((s) => !stale && (calibration = s))
      .catch(() => !stale && (calibration = null));
    return () => (stale = true);
  });
  const calText = $derived.by(() => {
    if (!calibration) return null;
    if (calibration.running) return `Calibrating (${calibration.running}) · ${Math.round(calibration.progress * 100)}%`;
    if (calibration.candidate) return "A calibration result is waiting to be applied";
    const parts = Object.entries(calibration.parts).filter(([, p]) => p?.active);
    if (parts.length === 0) return "Not calibrated";
    return `Calibrated: ${parts.map(([name, p]) => `${name} ${Math.round((p?.confidence ?? 0) * 100)}%`).join(", ")}`;
  });
  const calTone = $derived(!calibration ? "none" : calibration.failed ? "err" : calText === "Not calibrated" || calibration.candidate ? "warn" : "ok");
</script>

<section class="panel imu" aria-label="IMU">
  <div class="panel-head">
    <h2 class="panel-title">IMU</h2>
    <span class="text-[11px] text-fg-faint">{fused ? "fused on the board" : "live"}</span>
  </div>
  {#if series}
    <ImuView {series} {fused} {light} compact />
    <div class="axes mono">
      <span class="k">accel g</span>{#each accel as v, i (i)}<span>{AXES[i]} {fmt(v, 2)}</span>{/each}
      <span class="k">gyro °/s</span>{#each gyro as v, i (i)}<span>{AXES[i]} {fmt(v, 1)}</span>{/each}
    </div>
  {:else}
    <p class="text-[12.5px] text-fg-faint">Waiting for the IMU's readings…</p>
  {/if}
  <div class="cal">
    <span class="dot {calTone}"></span>
    <span class="truncate" title={calText ?? "Calibration unknown"}>{calText ?? "Calibration unknown"}</span>
    {#if ui.mode === "pit"}
      <button type="button" class="link ml-auto shrink-0 text-[12px]" onclick={() => oncalibrate?.()}>Calibrate…</button>
    {/if}
  </div>
</section>

<style>
  .imu {
    gap: 6px;
  }
  .axes {
    display: grid;
    grid-template-columns: 60px repeat(3, minmax(0, 1fr));
    gap: 2px 6px;
    font-size: 11.5px;
    white-space: nowrap;
  }
  .axes .k {
    color: var(--fg-faint);
    font-family: var(--font-ui);
  }
  .cal {
    display: flex;
    align-items: center;
    gap: 7px;
    min-width: 0;
    padding-top: 4px;
    border-top: 1px solid var(--hairline);
    font-size: 12px;
  }
  .dot {
    width: 7px;
    height: 7px;
    flex: none;
    border-radius: 50%;
    background: #3a414c;
  }
  .dot.ok {
    background: var(--ok);
  }
  .dot.warn {
    background: var(--warn);
  }
  .dot.err {
    background: var(--err);
  }
</style>
