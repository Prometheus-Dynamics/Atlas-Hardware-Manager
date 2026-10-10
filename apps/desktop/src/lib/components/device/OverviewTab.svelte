<script lang="ts">
  import { api, errorText, keyString, openExternal, type DeviceAction, type DeviceRecord } from "#lib/api/client.ts";
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
  import CameraPreview from "./CameraPreview.svelte";
  import ControlsCard from "./ControlsCard.svelte";
  import Disclosure from "./Disclosure.svelte";
  import FactGrid from "./FactGrid.svelte";
  import HealthCard from "./HealthCard.svelte";
  import LiveStats from "./LiveStats.svelte";
  import NowCard from "./NowCard.svelte";
  import SelfTestCard from "./SelfTestCard.svelte";

  let { record }: { record: DeviceRecord } = $props();

  const NAMES: Record<string, string> = { os: "OS", os_version: "OS version", device_package: "Device package", contract: "Contract" };
  const nice = (name: string) => NAMES[name] ?? name.replaceAll("_", " ");

  const attrs = $derived(record.identity.attributes ?? {});
  const manageUrl = $derived(attrs.manage_url ?? null);
  const online = $derived(record.presence === "online");
  const stream = $derived(online ? (attrs.camera_stream ?? null) : null);
  const telemetry = $derived(online && record.capabilities.includes("telemetry"));
  const reports = $derived(hasStatus(record));
  const id = $derived(keyString(record.key));
  const status = $derived(deviceStatus.byDevice.get(id));
  const statusError = $derived(deviceStatus.errors.get(id));

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

  const facts = $derived(
    [
      { label: "OS", value: attrs.os ? `${attrs.os}${attrs.os_version ? ` ${attrs.os_version}` : ""}` : null },
      { label: "Version", value: primaryVersion(record.identity), mono: true },
      { label: "Revision", value: attrs.revision ?? null },
      { label: "Hostname", value: attrs.hostname ?? null, mono: true },
      { label: "Storage", value: attrs.storage ?? null },
      { label: "Chip", value: attrs.chip ?? null },
      { label: "Connected by", value: linkText(record.link_kind, devices.nameOf) },
      { label: "Last seen", value: record.presence === "online" ? "Now" : timeAgo(record.last_seen_ms, clock.now) },
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
    } catch (error) {
      toasts.error(errorText(error));
    } finally {
      savingRobot = false;
    }
  }
</script>

<!-- Wide: the board's state on the left (Now, health, live, self-test), the
     camera, controls and details on the right; narrow: one column. -->
<div class="overview">
  <div class="col">
  {#if reports}
    <section class="flex flex-col gap-2.5">
      <h3 class="text-[12px] font-medium uppercase tracking-[0.06em] text-fg-faint">Now</h3>
      {#if status}
        <NowCard {status} {actions} {run} />
      {:else if statusError}
        <p class="text-[13px] text-warn-fg">{statusError}</p>
      {:else}
        <Skeleton height={58} />
      {/if}
    </section>
    {#if status}<HealthCard {status} readAt={deviceStatus.readAt.get(id)} />{/if}
  {/if}

  {#if telemetry}
    <LiveStats {record} />
  {/if}

  <SelfTestCard {record} />
  </div>

  <div class="col">
  {#if stream}
    <div class="camera">
      <CameraPreview src={stream} name={record.label ?? record.identity.name ?? record.key.serial} />
    </div>
  {/if}

  {#if online && actions.length > 0}
    <ControlsCard {actions} {run} update={reports ? (status?.update ?? null) : undefined} />
  {/if}

  <FactGrid {facts} />

  {#if manageUrl && !online}
    <div class="flex items-center gap-3">
      <Button icon="world-www" iconRight="external-link" onclick={() => openExternal(manageUrl)}>Open web UI</Button>
      <span class="mono truncate text-[12px] text-fg-faint">{manageUrl}</span>
    </div>
  {/if}

  <div class="grid grid-cols-2 gap-3">
    <Field label="Name in Atlas" hint="Enter or leaving the field saves it.">
      <input
        class="input"
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

  {#if Object.keys(record.identity.versions).length > 1}
    <Disclosure title="Versions" count={Object.keys(record.identity.versions).length} open>
      <FactGrid facts={Object.entries(record.identity.versions).map(([k, v]) => ({ label: nice(k), value: v, mono: true }))} plain />
    </Disclosure>
  {/if}

  {#if macs.length > 0}
    <Disclosure title="Network addresses" count={macs.length}>
      <FactGrid facts={macs.map(([k, v]) => ({ label: k.slice(4), value: v, mono: true }))} plain />
    </Disclosure>
  {/if}

  {#if extra.length > 0}
    <Disclosure title="More details" count={extra.length}>
      <FactGrid facts={extra.map(([k, v]) => ({ label: nice(k), value: v }))} plain />
    </Disclosure>
  {/if}

  <Disclosure title="Identity">
    <FactGrid facts={identity} plain />
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
  </div>
</div>

<style>
  .overview {
    display: grid;
    gap: 16px;
    align-items: start;
  }
  .col {
    display: flex;
    flex-direction: column;
    gap: 16px;
    min-width: 0;
  }
  @container (min-width: 1000px) {
    .overview {
      grid-template-columns: minmax(0, 1.3fr) minmax(0, 1fr);
    }
  }
  /* The camera at a useful size, not the whole width. */
  .camera {
    max-width: 640px;
  }
</style>
