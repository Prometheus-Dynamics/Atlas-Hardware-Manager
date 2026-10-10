<script lang="ts">
  // The IMU's orientation in 3D, live: the board's case with its axes
  // (x red, y green, z blue). With the board's own fusion (`fused`, lemnosd's
  // orientation device: the IMU and the magnetometer, calibrated) it shows
  // that; otherwise a filter of the accelerometer and gyro here (imu.ts),
  // whose heading drifts slowly. "Zero heading" makes the current heading
  // straight ahead either way.
  import Button from "#lib/components/common/Button.svelte";
  import { onMount } from "svelte";
  import CaseModel from "./CaseModel.svelte";
  import { FusedOrientation, Orientation, cssMatrix, euler } from "./imu.ts";
  import type { LiveSeries } from "./live.ts";

  /** fill: as tall as its container (a column with room to spare). */
  let {
    series,
    fused = null,
    fill = false,
  }: {
    series: LiveSeries;
    /** The board's orientation device's stream, when it has one. */
    fused?: LiveSeries | null;
    fill?: boolean;
  } = $props();

  const NAMES = ["acceleration.x", "acceleration.y", "acceleration.z", "angular_rate.x", "angular_rate.y", "angular_rate.z"];
  const indices = $derived(NAMES.map((name) => series.channels.findIndex((c) => c.name === name)));
  const usable = $derived(indices.every((i) => i >= 0));

  let orientation: Orientation | null = null;
  let board: FusedOrientation | null = null;
  const fusedChannels = $derived(fused ? FusedOrientation.channelsOf(fused.channels.map((c) => c.name)) : null);
  /** What the view shows now: the board's fusion once it streams. */
  let source = $state<"board" | "here">("here");
  let disturbed = $state(false);
  let confidence = $state<number | null>(null);
  const current = () => (source === "board" ? board : orientation);
  /** The case model didn't load (no WebGL, no model): the plain block instead. */
  let plain = $state(false);
  let matrix = $state("none");
  let angles = $state({ roll: 0, pitch: 0, yaw: 0 });
  let accelG = $state(0);
  let rateDps = $state(0);

  onMount(() => {
    let frame = 0;
    const draw = () => {
      if (usable) {
        orientation ??= new Orientation(indices);
        orientation.update(series);
        if (fused && fusedChannels && fused.count > 0) {
          board ??= new FusedOrientation(fusedChannels);
          board.update(fused);
          source = "board";
          disturbed = board.disturbed;
          confidence = board.confidence;
        } else source = "here";
        const q = current()?.q ?? orientation.q;
        matrix = cssMatrix(q);
        angles = euler(q);
        const at = series.count ? series.index(series.count - 1) : -1;
        if (at >= 0) {
          const [ax, ay, az, gx, gy, gz] = indices.map((c) => series.v[c][at]);
          accelG = Math.hypot(ax, ay, az) / 9.80665;
          rateDps = (Math.hypot(gx, gy, gz) * 180) / Math.PI;
        }
      }
      frame = requestAnimationFrame(draw);
    };
    frame = requestAnimationFrame(draw);
    return () => cancelAnimationFrame(frame);
  });

  const fmt = (value: number, digits = 1) => (Number.isFinite(value) ? value.toFixed(digits) : "–");
</script>

{#if usable}
  <div class="imu" class:fill>
    <div class="stage" aria-label="The IMU's orientation in 3D" role="img">
      {#if !plain}
        <CaseModel orientation={() => current()?.q ?? [1, 0, 0, 0]} onfail={() => (plain = true)} />
      {:else}
      <div class="scene">
        <div class="floor"></div>
        <div class="body" style="transform: {matrix}">
          <div class="face top"><span class="front">+x</span></div>
          <div class="face bottom"></div>
          <div class="face side front-side"></div>
          <div class="face side back-side"></div>
          <div class="face end left-end"></div>
          <div class="face end right-end"></div>
          <div class="axis x"><span>x</span></div>
          <div class="axis y"><span>y</span></div>
          <div class="axis z"><span>z</span></div>
        </div>
      </div>
      {/if}
    </div>
    <div class="readout">
      <dl>
        <div><dt>Roll</dt><dd>{fmt(angles.roll)}°</dd></div>
        <div><dt>Pitch</dt><dd>{fmt(angles.pitch)}°</dd></div>
        <div><dt>Heading</dt><dd>{fmt(angles.yaw)}°</dd></div>
        <div><dt>|a|</dt><dd>{fmt(accelG, 2)} g</dd></div>
        <div><dt>|ω|</dt><dd>{fmt(rateDps)} °/s</dd></div>
      </dl>
      <Button size="sm" variant="ghost" icon="refresh" action={async () => current()?.zero()}>Zero heading</Button>
      <p class="note">
        {#if source === "board"}
          Fused on the board{confidence !== null ? ` (IMU confidence ${Math.round(confidence * 100)}%)` : ""};
          {disturbed ? "heading from the gyro: the magnetometer isn't trusted now (calibrate it, or a magnet is near)" : "heading from the magnetometer"}.
        {:else}
          Tilt from gravity; heading from the gyro alone, so it drifts slowly.
        {/if}
      </p>
    </div>
  </div>
{/if}

<style>
  .imu {
    display: grid;
    grid-template-columns: minmax(0, 1fr) auto;
    gap: 12px;
    align-items: center;
    container-type: inline-size;
  }
  @media (max-width: 560px) {
    .imu {
      grid-template-columns: 1fr;
    }
  }
  .imu.fill {
    height: 100%;
    align-items: stretch;
  }
  .imu.fill .readout {
    align-self: center;
  }
  .stage {
    height: 230px;
    perspective: 800px;
    display: flex;
    align-items: center;
    justify-content: center;
    overflow: hidden;
    border-radius: var(--r-md);
    background: color-mix(in srgb, var(--fg) 4%, transparent);
  }
  .imu.fill .stage {
    height: auto;
    min-height: 230px;
  }
  /* Looking down at the floor from the front, a little above. */
  .scene {
    position: relative;
    width: 0;
    height: 0;
    transform-style: preserve-3d;
    transform: rotateX(62deg) rotateZ(-30deg);
  }
  .floor {
    position: absolute;
    width: 260px;
    height: 260px;
    left: -130px;
    top: -130px;
    border-radius: 50%;
    background: radial-gradient(circle, color-mix(in srgb, var(--fg) 10%, transparent), transparent 70%);
    transform: translateZ(-60px);
  }
  .body {
    position: absolute;
    transform-style: preserve-3d;
  }
  /* A 160 × 100 × 14 px block: top, bottom, the long sides (±y), the ends (±x). */
  .face {
    position: absolute;
    border: 1px solid color-mix(in srgb, var(--fg) 30%, transparent);
    backface-visibility: visible;
  }
  .top,
  .bottom {
    width: 160px;
    height: 100px;
    left: -80px;
    top: -50px;
    border-radius: 4px;
  }
  .top {
    transform: translateZ(7px);
    background: linear-gradient(135deg, color-mix(in srgb, var(--accent) 45%, #1d2a22), color-mix(in srgb, var(--accent) 25%, #141a16));
  }
  .bottom {
    transform: translateZ(-7px);
    background: color-mix(in srgb, var(--fg) 14%, #111);
  }
  .side {
    width: 160px;
    height: 14px;
    left: -80px;
    top: -7px;
    background: color-mix(in srgb, var(--accent) 22%, #101410);
  }
  .front-side {
    transform: translateY(50px) rotateX(90deg);
  }
  .back-side {
    transform: translateY(-50px) rotateX(90deg);
  }
  .end {
    width: 14px;
    height: 100px;
    left: -7px;
    top: -50px;
    background: color-mix(in srgb, var(--accent) 18%, #101410);
  }
  .left-end {
    transform: translateX(-80px) rotateY(90deg);
  }
  /* The +x end, where the camera faces: marked. */
  .right-end {
    transform: translateX(80px) rotateY(90deg);
    background: color-mix(in srgb, var(--err) 55%, #101410);
  }
  .front {
    position: absolute;
    right: 6px;
    top: 50%;
    transform: translateY(-50%);
    font-size: 11px;
    font-weight: 700;
    color: var(--err);
  }
  .axis {
    position: absolute;
    left: 0;
    top: 0;
    height: 2px;
    width: 110px;
    transform-origin: 0 50%;
  }
  .axis span {
    position: absolute;
    right: -10px;
    top: -7px;
    font-size: 11px;
    font-weight: 700;
  }
  .axis.x {
    background: var(--err);
    color: var(--err);
  }
  /* The scene's y points at the viewer: the board's +y is up the page. */
  .axis.y {
    background: var(--ok);
    color: var(--ok);
    transform: rotateZ(-90deg);
  }
  .axis.z {
    background: var(--info);
    color: var(--info);
    transform: rotateY(-90deg);
  }
  .readout {
    display: flex;
    flex-direction: column;
    gap: 8px;
    min-width: 11rem;
  }
  dl {
    display: grid;
    grid-template-columns: auto auto;
    gap: 2px 14px;
    font-size: 12.5px;
    font-variant-numeric: tabular-nums;
  }
  dl div {
    display: contents;
  }
  dt {
    color: var(--fg-muted);
  }
  dd {
    text-align: right;
    font-weight: 600;
    color: var(--fg);
  }
  .note {
    font-size: 11px;
    color: var(--fg-faint);
    max-width: 14rem;
  }
</style>
