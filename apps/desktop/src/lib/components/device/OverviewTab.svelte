<script lang="ts">
  import { api, keyString, type DeviceAction, type DeviceRecord } from "#lib/api/client.ts";
  import Skeleton from "#lib/components/common/Skeleton.svelte";
  import { deviceName } from "#lib/format.ts";
  import { deviceStatus, hasStatus } from "#lib/stores/status.svelte.ts";
  import { toasts } from "#lib/stores/toasts.svelte.ts";
  import { cameraStreams } from "#lib/present.ts";
  import { ui } from "#lib/stores/ui.svelte.ts";
  import BoardHardwareCard from "./BoardHardwareCard.svelte";
  import { BoardLive } from "./boardLive.svelte.ts";
  import BootsCard from "./BootsCard.svelte";
  import CameraStreams from "./CameraStreams.svelte";
  import DeviceCharts from "./DeviceCharts.svelte";
  import EventsCard from "./EventsCard.svelte";
  import HealthRows from "./HealthRows.svelte";
  import ImuCard from "./ImuCard.svelte";
  import NetworkCard from "./NetworkCard.svelte";
  import NowCard from "./NowCard.svelte";
  import SelfTestCard from "./SelfTestCard.svelte";
  import SetupCard from "./SetupCard.svelte";
  import VisionCard from "./VisionCard.svelte";

  /** onhistory, onsettings: open those tabs (from Recent events, Setup, Calibrate). */
  let { record, onhistory, onsettings }: { record: DeviceRecord; onhistory?: () => void; onsettings?: () => void } = $props();


  const attrs = $derived(record.identity.attributes ?? {});
  const online = $derived(record.presence === "online");
  const streams = $derived(online ? cameraStreams(record) : []);
  const telemetry = $derived(online && record.capabilities.includes("telemetry"));
  const reports = $derived(hasStatus(record));
  const id = $derived(keyString(record.key));
  const status = $derived(deviceStatus.byDevice.get(id));
  const statusError = $derived(deviceStatus.errors.get(id));

  /** Something under way or gone wrong with an update: Now shows it. */
  const happening = $derived(!!status?.update && !["idle", "confirmed", "cancelled"].includes(status.update.state));

  // The board's status and history while it is shown.
  $effect(() => deviceStatus.watch(record));

  // The controls: whatever actions the device offers, by whichever way it is reached.
  let actions = $state<DeviceAction[]>([]);
  $effect(() => {
    const key = record.key;
    if (!online || !record.capabilities.includes("actions")) {
      actions = [];
      return;
    }
    let stale = false;
    api
      .deviceActions(key)
      .then((list) => !stale && (actions = list))
      .catch(() => !stale && (actions = []));
    return () => (stale = true);
  });

  const DONE: Record<string, string> = {
    locate: "is signalling. Look for it.",
    reboot: "is restarting.",
    "power-off": "is shutting down. It stays off until its power is cycled.",
    "usb-boot": "is restarting into USB boot; install an image from Software.",
    "set-clock": "has this computer's time.",
    "update.cancel": "cancelled its update.",
    "update.rollback": "is restarting into its previous version.",
  };

  async function run(action: DeviceAction) {
    await api.runDeviceAction(record.key, action.id);
    const done = DONE[action.id];
    toasts.success(done ? `${deviceName(record)} ${done}` : `${action.label}: sent to ${deviceName(record)}.`);
    void deviceStatus.refresh(record.key);
  }
  // The board's IMU, ring and supply: one live stream for the panels that draw them.
  const board = new BoardLive();
  $effect(() => board.follow(record, status?.hardware));
  $effect(() => () => board.close());
  const hardware = $derived(status?.hardware ?? null);
  const imu = $derived(BoardLive.find(hardware, "imu"));
  const fusion = $derived(BoardLive.find(hardware, "orientation"));
  const ringDevice = $derived(BoardLive.find(hardware, "light"));
  const power = $derived(BoardLive.find(hardware, "power-monitor") ?? BoardLive.find(hardware, "power"));
  const snapReading = (name: string) => power?.readings.find((r) => r.name === name)?.value ?? null;
  const volts = $derived(board.latest(power?.id, "voltage") ?? board.latest(power?.id, "bus_voltage") ?? snapReading("voltage") ?? snapReading("bus_voltage"));
  const amps = $derived(board.latest(power?.id, "current") ?? snapReading("current"));
  const voltSeries = $derived.by(() => {
    void board.tick;
    const series = power ? board.series.get(power.id) : undefined;
    if (!series?.count) return [];
    const c = series.channels.findIndex((ch) => ch.name === "voltage" || ch.name === "bus_voltage");
    if (c < 0) return [];
    const out: number[] = [];
    const n = Math.min(series.count, 600);
    for (let i = series.count - n; i < series.count; i += 5) {
      const v = series.v[c][series.index(i)];
      if (Number.isFinite(v)) out.push(v);
    }
    return out;
  });
  const controlKey = $derived(record.capabilities.includes("hardware-control") ? record.key : null);
</script>

<!-- Revision 1: one dense screen. What it sees (camera, latency, tags) and
     what happened; is it OK (health, a trend per line) and how it's set up;
     its body (the IMU in 3D, the ring, the supply, the USB ports). A device
     without a board status (an MCU) keeps its charts and network. -->
{#if reports}
  <div class="overview">
    <div class="col">
      {#if streams.length || online}<VisionCard {record} {streams} />{/if}
      {#if status && happening}<NowCard {status} {actions} {run} />{/if}
      <div class="events"><EventsCard {record} limit={status && happening ? 3 : 6} onmore={onhistory} /></div>
    </div>
    <div class="col">
      {#if status}
        <HealthRows {record} {status} {volts} {amps} {voltSeries} />
      {:else if statusError}
        <p class="text-[13px] text-warn-fg">{statusError}</p>
      {:else}
        <Skeleton height={190} />
      {/if}
      {#if online}<SetupCard {record} {onsettings} />{/if}
      <BootsCard {record} {status} />
    </div>
    <div class="col">
      {#if online && imu}
        <ImuCard
          imuId={imu.id}
          series={board.series.get(imu.id) ?? null}
          fused={fusion ? (board.series.get(fusion.id) ?? null) : null}
          light={ringDevice ? (board.series.get(ringDevice.id) ?? null) : null}
          tick={board.tick}
          deviceKey={controlKey}
          oncalibrate={onsettings}
        />
      {/if}
      {#if online && hardware}
        <BoardHardwareCard {hardware} live={board} deviceKey={controlKey} {volts} {amps} onchanged={() => void deviceStatus.refresh(record.key)} />
      {/if}
      {#if ui.mode === "pit"}<SelfTestCard {record} />{/if}
    </div>
  </div>
{:else}
  <div class="overview two">
    <div class="col">
      {#if streams.length}<CameraStreams {streams} device={deviceName(record)} single />{/if}
      {#if telemetry}<DeviceCharts {record} />{/if}
    </div>
    <div class="col">
      {#if online}<NetworkCard {record} />{/if}
      <SelfTestCard {record} />
      <div class="events"><EventsCard {record} onmore={onhistory} /></div>
    </div>
  </div>
{/if}

<style>
  .overview {
    display: grid;
    gap: 10px;
    align-items: stretch;
    flex: 1 1 auto;
  }
  .col {
    display: flex;
    flex-direction: column;
    gap: 10px;
    min-width: 0;
    min-height: 0;
  }
  /* Recent events take what's left, showing as many as fit. */
  .events {
    flex: 1;
    min-height: 64px;
    display: flex;
    flex-direction: column;
    overflow: hidden;
  }
  .events > :global(.panel) {
    flex: 1;
    min-height: 0;
    overflow: hidden;
  }
  @container (min-width: 600px) {
    .overview {
      grid-template-columns: minmax(0, 1fr) minmax(0, 1fr);
    }
  }
  @container (min-width: 900px) {
    /* The window's height, not the columns' content: Recent events gives
       up rows so the rest fits; below 520 px the tab scrolls instead. */
    .overview:not(.two) {
      flex: 1 1 0;
      min-height: 520px;
      grid-template-rows: minmax(0, 1fr);
      grid-template-columns: minmax(0, 1.3fr) minmax(0, 1fr) minmax(0, 0.95fr);
    }
  }
</style>
