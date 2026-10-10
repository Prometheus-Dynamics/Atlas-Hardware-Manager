<script lang="ts">
  // The board's hardware, one card per device. While the tab is shown, a
  // board that streams its readings (board-stream) sends them live, at each
  // device's rate (motion sensors 100 Hz), drawn as charts; otherwise each
  // reading keeps a short trend of the snapshots seen (one every 10 s).
  import { api, errorText, keyString, type DeviceKey, type HardwareFrame, type HardwareSnapshot } from "#lib/api/client.ts";
  import SegmentedControl from "#lib/components/common/SegmentedControl.svelte";
  import { clock } from "#lib/stores/clock.svelte.ts";
  import { onDestroy, untrack } from "svelte";
  import { SvelteMap } from "svelte/reactivity";
  import HardwareCard from "./HardwareCard.svelte";
  import { appendSample, sampleKey } from "./hardware.ts";
  import { LiveSeries, WINDOWS, periodFor } from "./live.ts";

  let {
    hardware,
    boardKey,
    deviceKey = null,
  }: {
    hardware: HardwareSnapshot;
    /** The board, for its live stream. */
    boardKey: DeviceKey;
    /** The board, when its devices take commands (`hardware-control`). */
    deviceKey?: DeviceKey | null;
  } = $props();

  const history = new SvelteMap<string, number[]>();
  /** When each sample was read (the snapshot's `at`, ms), per reading. */
  const times = new SvelteMap<string, number[]>();
  /** The `at` of the last snapshot sampled. */
  let sampled = 0;

  $effect(() => {
    const snap = hardware;
    if (snap.at === sampled) return;
    sampled = snap.at;
    untrack(() => {
      for (const device of snap.devices) {
        for (const reading of device.readings) {
          if (reading.value === null) continue;
          const key = sampleKey(device.id, reading.name);
          history.set(key, appendSample(history.get(key), reading.value));
          times.set(key, appendSample(times.get(key), snap.at * 1000));
        }
      }
    });
  });

  // Live: the devices that read, at their rate. Restarted only when that
  // list changes, not on every snapshot.
  const wanted = $derived(
    hardware.devices
      .filter((d) => d.status === "available" || d.status === "degraded")
      .map((d) => [d.id, periodFor(d.class)] as [string, number]),
  );
  const wantedKey = $derived(`${keyString(boardKey)} ${wanted.map((w) => w.join(":")).join(",")}`);
  const live = new SvelteMap<string, LiveSeries>();
  let liveState = $state<"connecting" | "live" | "none" | "stopped">("connecting");
  let liveReason = $state<string | null>(null);
  let windowKey = $state("10");
  const windowS = $derived(Number(windowKey));
  /** Bumped ~10 times a second while live, so the tiles show the newest values. */
  let tick = $state(0);
  const ticker = setInterval(() => {
    if (liveState === "live") tick += 1;
  }, 100);
  onDestroy(() => clearInterval(ticker));

  function onFrame(frame: HardwareFrame) {
    if (frame.type === "devices") {
      live.clear();
      for (const device of frame.devices) if (!device.missing && !device.refused) live.set(device.id, new LiveSeries(device));
      liveState = "live";
    } else if (frame.type === "samples") {
      const series = live.get(frame.device);
      if (series) for (const [t, values] of frame.samples) series.push(t, values);
    } else {
      liveState = "stopped";
      liveReason = frame.reason;
    }
  }

  $effect(() => {
    void wantedKey;
    const key = boardKey;
    const devices = untrack(() => wanted);
    if (devices.length === 0) {
      liveState = "none";
      return;
    }
    let stop: (() => void) | null = null;
    let gone = false;
    liveState = "connecting";
    liveReason = null;
    api
      .streamHardware(key, devices, (frame) => !gone && onFrame(frame))
      .then((stopper) => {
        if (gone) stopper?.();
        else if (stopper) stop = stopper;
        else liveState = "none";
      })
      .catch((error) => {
        liveState = "stopped";
        liveReason = errorText(error);
      });
    return () => {
      gone = true;
      stop?.();
      live.clear();
    };
  });

  const ago = $derived(Math.max(0, Math.round(clock.now / 1000 - hardware.at)));
  const windowOptions = WINDOWS.map((s) => ({ value: String(s), label: `${s} s` }));
</script>

<div class="flex flex-col gap-4">
  <div class="flex flex-wrap items-center justify-between gap-2">
    <p class="flex items-center gap-2 text-[12px] text-fg-faint">
      {#if liveState === "live"}
        <span class="live-dot"></span>Live from the board
      {:else if liveState === "connecting"}
        Connecting to the board's live readings…
      {:else if liveState === "stopped"}
        Live readings stopped{liveReason ? `: ${liveReason}` : ""}; showing the snapshot (read {ago} s ago)
      {:else}
        Snapshot, read {ago} s ago (this board doesn't stream its readings)
      {/if}
    </p>
    {#if liveState === "live"}
      <SegmentedControl options={windowOptions} bind:value={windowKey} label="Chart window" size="sm" />
    {/if}
  </div>
  {#each hardware.devices as device (device.id)}
    <HardwareCard
      {device}
      {deviceKey}
      live={live.get(device.id) ?? null}
      {windowS}
      {tick}
      seriesOf={(reading) => ({
        values: history.get(sampleKey(device.id, reading)) ?? [],
        times: times.get(sampleKey(device.id, reading)) ?? [],
      })}
    />
  {/each}
</div>

<style>
  .live-dot {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: var(--ok);
  }
</style>
