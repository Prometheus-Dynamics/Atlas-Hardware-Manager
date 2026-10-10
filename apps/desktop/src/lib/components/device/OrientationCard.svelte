<script lang="ts">
  // The board's orientation on its Overview: the IMU's live stream (50 Hz,
  // only while shown) drawn as the 3D case, for boards that stream readings
  // and have an IMU.
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
  let series = $state<LiveSeries | null>(null);

  $effect(() => {
    const id = imuId;
    const key = record.key;
    if (!id || record.presence !== "online") return;
    let stop: (() => void) | null = null;
    let gone = false;
    void api
      .streamHardware(key, [[id, 20]], (frame: HardwareFrame) => {
        if (gone) return;
        if (frame.type === "devices") {
          const device = frame.devices.find((d) => d.id === id && !d.missing && !d.refused);
          if (device && (!series || series.channels.length !== device.channels.length)) series = new LiveSeries(device);
        } else if (frame.type === "samples" && frame.device === id && series) {
          for (const [t, values] of frame.samples) series.push(t, values);
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
    <div class="glass px-3 py-3" class:grow={fill}><ImuView {series} {fill} /></div>
  </section>
{/if}

<style>
  .grow {
    flex: 1;
    min-height: 0;
  }
</style>
