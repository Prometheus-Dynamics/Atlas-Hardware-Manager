<script lang="ts">
  // The board's orientation on its Overview: the IMU's live stream (50 Hz,
  // only while shown) drawn as the 3D case, for boards that stream readings
  // and have an IMU; with the board's own fusion (an `orientation` device)
  // streamed too, the view shows that.
  import { api, keyString, type DeviceRecord, type HardwareFrame } from "#lib/api/client.ts";
  import { deviceStatus } from "#lib/stores/status.svelte.ts";
  import ImuView from "./ImuView.svelte";
  import { LiveSeries } from "./live.ts";

  /** fill: grows to the height its column leaves (full screen). */
  let { record, fill = false }: { record: DeviceRecord; fill?: boolean } = $props();

  const imu = $derived(
    (deviceStatus.byDevice.get(keyString(record.key))?.hardware?.devices ?? []).find(
      (d) => d.class === "imu" && (d.status === "available" || d.status === "degraded"),
    ) ?? null,
  );
  const imuId = $derived(imu?.id ?? null);
  const fusionId = $derived(
    (deviceStatus.byDevice.get(keyString(record.key))?.hardware?.devices ?? []).find(
      (d) => d.class === "orientation" && d.status !== "missing",
    )?.id ?? null,
  );
  let series = $state<LiveSeries | null>(null);
  let fused = $state<LiveSeries | null>(null);
  const lightId = $derived(
    (deviceStatus.byDevice.get(keyString(record.key))?.hardware?.devices ?? []).find((d) => d.class === "light" && d.status !== "missing")?.id ?? null,
  );
  let light = $state<LiveSeries | null>(null);

  $effect(() => {
    const id = imuId;
    const fusion = fusionId;
    const ringId = lightId;
    const key = record.key;
    if (!id || record.presence !== "online") return;
    let stop: (() => void) | null = null;
    let gone = false;
    void api
      .streamHardware(key, [[id, 20] as [string, number], ...(fusion ? [[fusion, 20] as [string, number]] : []), ...(ringId ? [[ringId, 50] as [string, number]] : [])], (frame: HardwareFrame) => {
        if (gone) return;
        if (frame.type === "devices") {
          const device = frame.devices.find((d) => d.id === id && !d.missing && !d.refused);
          if (device && (!series || series.channels.length !== device.channels.length)) series = new LiveSeries(device);
          const board = fusion ? frame.devices.find((d) => d.id === fusion && !d.missing && !d.refused) : undefined;
          if (board && (!fused || fused.channels.length !== board.channels.length)) fused = new LiveSeries(board);
          const ringDevice = ringId ? frame.devices.find((d) => d.id === ringId && !d.missing && !d.refused) : undefined;
          if (ringDevice && (!light || light.channels.length !== ringDevice.channels.length)) light = new LiveSeries(ringDevice);
        } else if (frame.type === "samples" && frame.device === id && series) {
          for (const [t, values] of frame.samples) series.push(t, values);
        } else if (frame.type === "samples" && frame.device === fusion && fused) {
          for (const [t, values] of frame.samples) fused.push(t, values);
        } else if (frame.type === "samples" && frame.device === ringId && light) {
          for (const [t, values] of frame.samples) light.push(t, values);
        }
      })
      .then((stopper) => {
        if (gone) stopper?.();
        else stop = stopper;
      })
      .catch(() => {});
    return () => {
      gone = true;
      stop?.();
    };
  });
</script>

{#if imu && series}
  <section class="flex flex-col gap-2.5" class:grow={fill}>
    <h3 class="text-[12px] font-medium uppercase tracking-[0.06em] text-fg-faint">Orientation</h3>
    <div class="glass px-3 py-3" class:grow={fill}><ImuView {series} {fused} {light} {fill} /></div>
  </section>
{/if}

<style>
  .grow {
    flex: 1;
    min-height: 0;
  }
</style>
