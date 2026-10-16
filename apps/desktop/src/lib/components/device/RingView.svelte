<script lang="ts">
  // The status ring as it is: its LEDs in a circle, each in the colour the
  // board shows now (lemnosd's frames, as streamed), logical LED 0 at the
  // top running clockwise. A click shows the active look and the ring's
  // brightness, which can be set here.
  import { api, errorText, type DeviceKey } from "#lib/api/client.ts";
  import { toasts } from "#lib/stores/toasts.svelte.ts";
  import type { LiveSeries } from "./live.ts";

  let {
    series,
    deviceId,
    deviceKey = null,
    tick = 0,
    size = 64,
  }: { series: LiveSeries | null; deviceId: string; deviceKey?: DeviceKey | null; tick?: number; size?: number } = $props();

  /** Logical order (top first, clockwise), as 0xRRGGBB with white folded in, and the brightness. */
  const frame = $derived.by(() => {
    void tick;
    if (!series?.count) return null;
    const at = series.index(series.count - 1);
    const leds: number[] = [];
    let brightness: number | null = null;
    series.channels.forEach((channel, c) => {
      const v = series.v[c][at];
      if (channel.name === "brightness" && Number.isFinite(v)) brightness = v;
      if (!channel.name.startsWith("led.")) return;
      const i = Number(channel.name.slice(4));
      const wrgb = Number.isFinite(v) ? v >>> 0 : 0;
      const w = (wrgb >>> 24) & 0xff;
      const add = (x: number) => Math.min(255, x + w);
      leds[i] = (add((wrgb >> 16) & 0xff) << 16) | (add((wrgb >> 8) & 0xff) << 8) | add(wrgb & 0xff);
    });
    return { leds, brightness };
  });
  const n = $derived(frame?.leds.length ?? 0);
  const css = (rgb: number) => (rgb ? `#${rgb.toString(16).padStart(6, "0")}` : "#20262e");
  const lit = $derived(frame ? frame.leds.filter((c) => c).length : 0);

  let open = $state(false);
  let look = $state<string | null>(null);
  let setTo = $state<number | null>(null);
  const brightness = $derived(setTo ?? frame?.brightness ?? null);

  async function show() {
    open = !open;
    if (!open || !deviceKey) return;
    try {
      const text = String((await api.hardwareAction(deviceKey, deviceId, "looks.preset.list")).text ?? "");
      look = text.split("\n").find((l) => l.startsWith("*"))?.replace(/^\*\s*/, "").trim() ?? null;
    } catch {
      look = null;
    }
  }
  async function setBrightness(value: number) {
    if (!deviceKey) return;
    setTo = value;
    try {
      await api.hardwareAction(deviceKey, deviceId, "light.brightness", { value: Math.round(value * 1000) / 1000 });
    } catch (error) {
      setTo = null;
      toasts.error(errorText(error));
    }
  }
</script>

<div class="ring">
  <button type="button" class="face" onclick={() => void show()} title="The status ring now: {lit} of {n} lit. Click for its look and brightness." aria-expanded={open}>
    <svg viewBox="-50 -50 100 100" width={size} height={size} role="img" aria-label="Status ring, {lit} of {n} LEDs lit">
      <circle r="38" fill="none" stroke="#1a1f26" stroke-width="14" />
      {#each frame?.leds ?? [] as rgb, i (i)}
        {@const a = (i / n) * 2 * Math.PI - Math.PI / 2}
        <circle cx={Math.cos(a) * 38} cy={Math.sin(a) * 38} r={Math.max(2.5, Math.min(6, 110 / n))} fill={css(rgb)} stroke="#0f1115" stroke-width="1" />
      {/each}
      {#if !frame}<text y="4" text-anchor="middle" font-size="11" fill="#858d98">no frames</text>{/if}
    </svg>
  </button>
  {#if open}
    <div class="pop" role="dialog" aria-label="Status ring">
      <div class="row"><span class="k">Look</span><span class="v truncate" title={look ?? "unknown"}>{look ?? "—"}</span></div>
      <label class="row">
        <span class="k">Brightness</span>
        <input type="range" min="0" max="1" step="0.05" value={brightness ?? 0.4} disabled={!deviceKey} onchange={(e) => void setBrightness(Number(e.currentTarget.value))} />
        <span class="v mono">{brightness === null ? "—" : `${Math.round(brightness * 100)}%`}</span>
      </label>
      <span class="note">{n} LEDs · {lit} lit · more in Settings</span>
    </div>
  {/if}
</div>

<style>
  .ring {
    position: relative;
    display: inline-flex;
  }
  .face {
    display: grid;
    place-items: center;
    border-radius: 50%;
  }
  .face:hover {
    background: var(--glass-hover);
  }
  .pop {
    position: absolute;
    left: 0;
    top: calc(100% + 4px);
    z-index: 30;
    width: 230px;
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 8px 10px;
    border: 1px solid var(--glass-border);
    border-radius: var(--r-panel);
    background: var(--glass);
    box-shadow: 0 8px 22px rgba(0, 0, 0, 0.5);
  }
  .row {
    display: grid;
    grid-template-columns: 70px minmax(0, 1fr) auto;
    align-items: center;
    gap: 6px;
    font-size: 12px;
  }
  .row input {
    width: 100%;
    min-width: 0;
    accent-color: var(--accent);
  }
  .k {
    color: var(--fg-faint);
  }
  .v {
    min-width: 0;
  }
  .note {
    font-size: 11px;
    color: var(--fg-faint);
  }
</style>
