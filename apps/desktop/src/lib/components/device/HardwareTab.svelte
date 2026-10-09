<script lang="ts">
  // The board's hardware, one card per device. Each reading keeps a short
  // trend of the values seen while this tab is open; a new snapshot (its
  // `at`) adds a point.
  import type { DeviceKey, HardwareSnapshot } from "#lib/api/client.ts";
  import { clock } from "#lib/stores/clock.svelte.ts";
  import { untrack } from "svelte";
  import { SvelteMap } from "svelte/reactivity";
  import HardwareCard from "./HardwareCard.svelte";
  import { appendSample, sampleKey } from "./hardware.ts";

  let {
    hardware,
    deviceKey = null,
  }: {
    hardware: HardwareSnapshot;
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

  const ago = $derived(Math.max(0, Math.round(clock.now / 1000 - hardware.at)));
</script>

<div class="flex flex-col gap-4">
  {#each hardware.devices as device (device.id)}
    <HardwareCard {device} {deviceKey} seriesOf={(reading) => ({
        values: history.get(sampleKey(device.id, reading)) ?? [],
        times: times.get(sampleKey(device.id, reading)) ?? [],
      })} />
  {/each}
  <p class="text-[12px] text-fg-faint">Read {ago} s ago</p>
</div>
