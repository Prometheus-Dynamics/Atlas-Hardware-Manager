<script lang="ts">
  import { api, errorText, keyString, type DeviceAction, type DeviceRecord } from "#lib/api/client.ts";
  import Button from "#lib/components/common/Button.svelte";
  import ConfirmButton from "#lib/components/common/ConfirmButton.svelte";
  import Field from "#lib/components/common/Field.svelte";
  import Skeleton from "#lib/components/common/Skeleton.svelte";
  import { clockTime, deviceName, linkText, primaryVersion, timeAgo } from "#lib/format.ts";
  import { clock } from "#lib/stores/clock.svelte.ts";
  import { devices } from "#lib/stores/devices.svelte.ts";
  import { robots } from "#lib/stores/robots.svelte.ts";
  import { deviceStatus, hasStatus } from "#lib/stores/status.svelte.ts";
  import { toasts } from "#lib/stores/toasts.svelte.ts";
  import { cameraStreams } from "#lib/present.ts";
  import CameraStreams from "./CameraStreams.svelte";
  import OrientationCard from "./OrientationCard.svelte";
  import ControlsCard from "./ControlsCard.svelte";
  import UsbRouteCard from "./UsbRouteCard.svelte";
  import Disclosure from "./Disclosure.svelte";
  import FactGrid from "./FactGrid.svelte";
  import HealthCard from "./HealthCard.svelte";
  import DeviceCharts from "./DeviceCharts.svelte";
  import BootsCard from "./BootsCard.svelte";
  import EventsCard from "./EventsCard.svelte";
  import NetworkCard from "./NetworkCard.svelte";
  import NowCard from "./NowCard.svelte";
  import SelfTestCard from "./SelfTestCard.svelte";
  import WebUiButton from "./WebUiButton.svelte";

  /** onhistory: opens the History tab (Recent events' link). */
  let { record, onhistory }: { record: DeviceRecord; onhistory?: () => void } = $props();

  const NAMES: Record<string, string> = { os: "OS", os_version: "OS version", device_package: "Device package", contract: "Contract" };
  const nice = (name: string) => NAMES[name] ?? name.replaceAll("_", " ");

  const attrs = $derived(record.identity.attributes ?? {});
  const manageUrl = $derived(attrs.manage_url ?? null);
  const online = $derived(record.presence === "online");
  const streams = $derived(online ? cameraStreams(record) : []);
  const telemetry = $derived(online && record.capabilities.includes("telemetry"));
  const reports = $derived(hasStatus(record));
  const id = $derived(keyString(record.key));
  const status = $derived(deviceStatus.byDevice.get(id));
  const statusError = $derived(deviceStatus.errors.get(id));

  let width = $state(0);
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
  const shown = new Set(["os", "os_version", "revision", "hostname", "manage_url", "recovery_steps", "storage", "model", "camera_stream"]);
  const macs = $derived(Object.entries(attrs).filter(([name]) => name.startsWith("mac.")));
  const extra = $derived(Object.entries(attrs).filter(([name]) => !shown.has(name) && !name.startsWith("mac.")));

  // What the header (model, version, storage, serial) and Network
  // (addresses, hostname, link) don't already say.
  const facts = $derived(
    [
      { label: "OS", value: attrs.os ? `${attrs.os}${attrs.os_version ? ` ${attrs.os_version}` : ""}` : null },
      { label: "Revision", value: attrs.revision ?? null },
      { label: "Chip", value: attrs.chip ?? null },
      { label: "Connected by", value: online ? null : linkText(record.link_kind, devices.nameOf) },
      { label: "Last seen", value: online ? null : timeAgo(record.last_seen_ms, clock.now) },
    ].filter((f) => f.value),
  );

  const identity = $derived([
    { label: "Family", value: record.key.family, mono: true },
    { label: "Serial", value: record.key.serial, mono: true },
    { label: "Model", value: record.identity.model },
    { label: "Name reported", value: record.identity.name ?? "—" },
    { label: "Address", value: record.identity.address, mono: true },
    { label: "Link id", value: record.identity.link, mono: true },
    { label: "First seen", value: clockTime(record.first_seen_ms) },
    { label: "Capabilities", value: record.capabilities.join(", ") },
  ]);

  // svelte-ignore state_referenced_locally
  let label = $state(record.label ?? "");
  let savingLabel = $state(false);
  let savingRobot = $state(false);

  async function saveLabel() {
    const next = label.trim() || null;
    if (next === record.label || savingLabel) return;
    savingLabel = true;
    try {
      await api.setDeviceLabel(record.key, next);
      toasts.success(next ? `Renamed to "${next}".` : "Name cleared.");
    } catch (error) {
      toasts.error(errorText(error));
      label = record.label ?? "";
    } finally {
      savingLabel = false;
    }
  }

  async function setRobot(value: string) {
    savingRobot = true;
    try {
      await api.setDeviceRobot(record.key, value || null);
      toasts.success(value ? `${deviceName(record)} is on ${value}.` : `${deviceName(record)} is on no robot.`);
    } catch (error) {
      toasts.error(errorText(error));
    } finally {
      savingRobot = false;
    }
  }
</script>

<!-- Three columns as the v2 design: what it sees and how it is (camera,
     status, details); its readings, network and recent events; its body
     (orientation, boots, self-test, controls). Each reading shows once.
     Narrower windows fold to two columns, then one. -->
{#snippet detailsCard()}
    <section class="panel" aria-label="Details">
      <div class="panel-head"><h2 class="panel-title">Details</h2></div>
      {#if facts.length > 0}<FactGrid {facts} plain />{/if}

      {#if manageUrl && !online}
        <div class="flex items-center gap-3">
          <WebUiButton {record} size="md" label="Open web UI" />
          <span class="mono truncate text-[12px] text-fg-faint">{manageUrl}</span>
        </div>
      {/if}

      <div class="grid grid-cols-2 gap-3">
        <Field label="Name in Atlas">
          <input
            class="input"
            title="Enter or leaving the field saves it."
            bind:value={label}
            placeholder={record.identity.name ?? "Give this device a name"}
            onkeydown={(e) => {
              if (e.key === "Enter") void saveLabel();
              if (e.key === "Escape") label = record.label ?? "";
            }}
            onblur={saveLabel}
            disabled={savingLabel}
          />
        </Field>
        <Field label="Robot">
          <select class="select" value={record.robot ?? ""} onchange={(e) => setRobot(e.currentTarget.value)} disabled={savingRobot}>
            <option value="">No robot</option>
            {#each robots.names as name (name)}
              <option value={name}>{name}</option>
            {/each}
          </select>
        </Field>
      </div>

      {#if online && actions.length > 0}
        <ControlsCard {actions} {run} update={reports ? (status?.update ?? null) : undefined} />
      {/if}

      <Disclosure title="Identity, versions and addresses">
        <div class="flex flex-col gap-3">
          <FactGrid facts={identity} plain />
          {#if Object.keys(record.identity.versions).length > 1}
            <FactGrid facts={Object.entries(record.identity.versions).map(([k, v]) => ({ label: nice(k), value: v, mono: true }))} plain />
          {/if}
          {#if macs.length > 0}
            <FactGrid facts={macs.map(([k, v]) => ({ label: k.slice(4), value: v, mono: true }))} plain />
          {/if}
          {#if extra.length > 0}
            <FactGrid facts={extra.map(([k, v]) => ({ label: nice(k), value: v }))} plain />
          {/if}
        </div>
      </Disclosure>

      {#if record.presence === "offline"}
        <div class="glass flex flex-col gap-3 px-4 py-3.5">
          <p class="text-[13px] text-fg-muted">
            Forgetting removes this device, its name, and its robot assignment. It comes back as new if it's seen again.
          </p>
          <div>
            <ConfirmButton action={() => api.forgetDevice(record.key)} icon="trash" prompt="Forget this device?" confirmLabel="Forget">
              Forget device
            </ConfirmButton>
          </div>
        </div>
      {/if}
    </section>
{/snippet}

<div class="overview" class:two={!reports} bind:clientWidth={width}>
  <div class="col">
    {#if streams.length}
      <div class="camera" class:several={streams.length > 1}>
        <CameraStreams {streams} device={deviceName(record)} single />
      </div>
    {/if}

    {#if reports}
      {#if status}
        {#if happening}<NowCard {status} {actions} {run} />{/if}
        <HealthCard {status} readAt={deviceStatus.readAt.get(id)} thermal={!telemetry} />
      {:else if statusError}
        <p class="text-[13px] text-warn-fg">{statusError}</p>
      {:else}
        <Skeleton height={58} />
      {/if}
    {/if}
    <!-- A device without a board status (an MCU): its charts here, the rest beside. -->
    {#if !reports && telemetry}<DeviceCharts {record} />{/if}

  </div>

  <div class="col">
    {#if telemetry && reports}<DeviceCharts {record} />{/if}
    {#if online}
      <NetworkCard {record} />
      <UsbRouteCard {record} {actions} {run} />
    {/if}
    {#if !reports}
      <SelfTestCard {record} />
      {@render detailsCard()}
      <div class="events"><EventsCard {record} onmore={onhistory} /></div>
    {/if}
    {#if reports}
      <div class="events"><EventsCard {record} onmore={onhistory} /></div>
    {/if}
  </div>

  {#if reports}
  <div class="col">
    {#if online}<OrientationCard {record} fill />{/if}
    <BootsCard {record} {status} />
    <SelfTestCard {record} />
    {@render detailsCard()}
  </div>
  {/if}
</div>

<style>
  .overview {
    display: grid;
    gap: 12px;
    align-items: stretch;
    flex: 1 1 auto;
  }
  .col {
    display: flex;
    flex-direction: column;
    gap: 12px;
    min-width: 0;
    min-height: 0;
  }
  .events {
    flex: 1;
    min-height: 160px;
    display: flex;
    flex-direction: column;
  }
  @container (min-width: 760px) {
    .overview {
      grid-template-columns: minmax(0, 1fr) minmax(0, 1fr);
    }
  }
  @container (min-width: 980px) {
    .overview:not(.two) {
      grid-template-columns: minmax(0, 1.2fr) minmax(0, 1fr) minmax(0, 1fr);
    }
  }
</style>
