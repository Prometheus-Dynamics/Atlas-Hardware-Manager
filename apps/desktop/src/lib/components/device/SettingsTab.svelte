<script lang="ts">
  // A device's settings: its name and robot, how it's reached and its USB
  // route, its identity, and the board's hardware with their controls and
  // calibration. In ROBOT mode the network settings are locked (switch to
  // PIT to change them); LEDs and the like stay live.
  import { api, errorText, type DeviceAction, type DeviceRecord, type HardwareSnapshot } from "#lib/api/client.ts";
  import ConfirmButton from "#lib/components/common/ConfirmButton.svelte";
  import Field from "#lib/components/common/Field.svelte";
  import { clockTime, deviceName, linkText, timeAgo } from "#lib/format.ts";
  import { clock } from "#lib/stores/clock.svelte.ts";
  import { devices } from "#lib/stores/devices.svelte.ts";
  import { robots } from "#lib/stores/robots.svelte.ts";
  import { deviceStatus } from "#lib/stores/status.svelte.ts";
  import { toasts } from "#lib/stores/toasts.svelte.ts";
  import { ui } from "#lib/stores/ui.svelte.ts";
  import Disclosure from "./Disclosure.svelte";
  import FactGrid from "./FactGrid.svelte";
  import HardwareTab from "./HardwareTab.svelte";
  import NetworkCard from "./NetworkCard.svelte";
  import UsbRouteCard from "./UsbRouteCard.svelte";

  let { record, hardware }: { record: DeviceRecord; hardware: HardwareSnapshot | null } = $props();

  const NAMES: Record<string, string> = { os: "OS", os_version: "OS version", device_package: "Device package", contract: "Contract" };
  const nice = (name: string) => NAMES[name] ?? name.replaceAll("_", " ");
  const attrs = $derived(record.identity.attributes ?? {});
  const online = $derived(record.presence === "online");
  const locked = $derived(ui.mode === "robot");

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
  async function run(action: DeviceAction) {
    await api.runDeviceAction(record.key, action.id);
    toasts.success(`${action.label}: sent to ${deviceName(record)}.`);
    void deviceStatus.refresh(record.key);
  }

  const shown = new Set(["os", "os_version", "revision", "hostname", "manage_url", "recovery_steps", "storage", "model", "camera_stream"]);
  const macs = $derived(Object.entries(attrs).filter(([name]) => name.startsWith("mac.")));
  const extra = $derived(Object.entries(attrs).filter(([name]) => !shown.has(name) && !name.startsWith("mac.")));
  const facts = $derived(
    [
      { label: "OS", value: attrs.os ? `${attrs.os}${attrs.os_version ? ` ${attrs.os_version}` : ""}` : null },
      { label: "Revision", value: attrs.revision ?? null },
      { label: "Chip", value: attrs.chip ?? null },
      { label: "Security switch", value: attrs.security ? (attrs.security === "on" ? "on: enrolled computers only" : "off (FRC default)") : null },
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

<div class="settings">
  <div class="col">
    <section class="panel" aria-label="Name and robot">
      <div class="panel-head"><h2 class="panel-title">In Atlas</h2></div>
      {#if facts.length > 0}<FactGrid {facts} plain />{/if}
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
    </section>

    {#if online}
      {#if locked}
        <p class="lock">ROBOT mode: the network settings are locked. Switch to PIT (the chip by the name) to change them.</p>
      {/if}
      <NetworkCard {record} />
      <fieldset class="contents" disabled={locked}><UsbRouteCard {record} {actions} {run} /></fieldset>
    {/if}

    <section class="panel" aria-label="Identity">
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
    </section>

    {#if record.presence === "offline"}
      <section class="panel flex flex-col gap-3">
        <p class="text-[13px] text-fg-muted">
          Forgetting removes this device, its name, and its robot assignment. It comes back as new if it's seen again.
        </p>
        <div>
          <ConfirmButton action={() => api.forgetDevice(record.key)} icon="trash" prompt="Forget this device?" confirmLabel="Forget">Forget device</ConfirmButton>
        </div>
      </section>
    {/if}
  </div>

  {#if hardware}
    <div class="col wide">
      <HardwareTab {hardware} boardKey={record.key} deviceKey={record.capabilities.includes("hardware-control") ? record.key : null} />
    </div>
  {/if}
</div>

<style>
  .settings {
    display: grid;
    gap: 12px;
    align-items: start;
  }
  .col {
    display: flex;
    flex-direction: column;
    gap: 12px;
    min-width: 0;
  }
  .lock {
    padding: 7px 10px;
    border: 1px solid #4a3a1f;
    border-radius: var(--r-panel);
    background: #1d1912;
    font-size: 12.5px;
    color: var(--warn-fg);
  }
  .contents {
    display: contents;
  }
  @container (min-width: 980px) {
    .settings {
      grid-template-columns: minmax(0, 1fr) minmax(0, 2fr);
    }
  }
</style>
