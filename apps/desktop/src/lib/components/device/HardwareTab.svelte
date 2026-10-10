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
    compact = false,
  }: {
    hardware: HardwareSnapshot;
    /** The board, for its live stream. */
    boardKey: DeviceKey;
    /** The board, when its devices take commands (`hardware-control`). */
    deviceKey?: DeviceKey | null;
    /** Beside other devices: charts only, for the devices that stream. */
    compact?: boolean;
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

  // Live: the devices that have readings, at their rate. Restarted only when
  // that list changes: not on every snapshot, and not when a device's status
  // flips (available, degraded), which used to clear every chart. The board
  // refuses what it can't stream.
  // The board's orientation (lemnosd's fusion) has no snapshot readings: it
  // computes only while someone subscribes, so it's asked for here.
  const wanted = $derived(
    hardware.devices
      .filter((d) => (d.readings.length > 0 || d.class === "orientation") && d.status !== "missing")
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
      // Sent again after a reconnect: keep a device's samples when its
      // channels are the same, so its charts carry on.
      const next = new Map<string, LiveSeries>();
      for (const device of frame.devices) {
        if (device.missing || device.refused) continue;
        const had = live.get(device.id);
        const same = had && had.channels.map((c) => c.name).join() === device.channels.map((c) => c.name).join();
        next.set(device.id, same ? had : new LiveSeries(device));
      }
      live.clear();
      for (const [id, series] of next) live.set(id, series);
      liveState = "live";
    } else if (frame.type === "samples") {
      const series = live.get(frame.device);
      if (series) for (const [t, values] of frame.samples) series.push(t, values);
    } else {
      liveState = "stopped";
      liveReason = frame.reason;
    }
  }

  /** The board the buffers belong to: another board starts them afresh. */
  let liveBoard = "";

  $effect(() => {
    void wantedKey;
    const key = boardKey;
    const devices = untrack(() => wanted);
    if (keyString(key) !== liveBoard) {
      liveBoard = keyString(key);
      live.clear();
    }
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
    };
  });

  // Compact (beside other devices): only what streams. The orientation is
  // drawn in the IMU's card, not a card of its own.
  const fusion = $derived(hardware.devices.find((d) => d.class === "orientation" && d.status !== "missing") ?? null);
  const cards = $derived(
    (compact ? hardware.devices.filter((d) => live.has(d.id)) : hardware.devices).filter((d) => d.class !== "orientation"),
  );
  const ago = $derived(Math.max(0, Math.round(clock.now / 1000 - hardware.at)));
  const windowOptions = WINDOWS.map((s) => ({ value: String(s), label: `${s} s` }));
</script>

<div class="flex flex-col gap-3">
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
  <div class="cards" class:compact>
  {#each cards as device (device.id)}
    <HardwareCard
      {compact}
      {device}
      {deviceKey}
      live={live.get(device.id) ?? null}
      fused={device.class === "imu" && fusion ? (live.get(fusion.id) ?? null) : null}
      {windowS}
      {tick}
      seriesOf={(reading) => ({
        values: history.get(sampleKey(device.id, reading)) ?? [],
        times: times.get(sampleKey(device.id, reading)) ?? [],
      })}
    />
  {/each}
  </div>
</div>

<style>
  /* Packed in columns as there is room (full screen), each card whole, the
     IMU with its 3D view across them all. */
  .cards {
    columns: 3 520px;
    column-gap: 12px;
    /* The last card's margin isn't space under the tab. */
    margin-bottom: -12px;
  }
  .cards > :global(*) {
    break-inside: avoid;
    margin-bottom: 12px;
  }
  .cards > :global(.wide) {
    column-span: all;
  }
  .cards.compact {
    columns: auto;
  }
  .live-dot {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: var(--ok);
  }
</style>
