<script lang="ts">
  import { api, errorText, openExternal, type DeviceRecord } from "$lib/api/client";
  import ConfirmButton from "$lib/components/common/ConfirmButton.svelte";
  import { clockTime, linkText, timeAgo } from "$lib/format";
  import { clock } from "$lib/stores/clock.svelte";
  import { devices } from "$lib/stores/devices.svelte";
  import { robots } from "$lib/stores/robots.svelte";
  import { toasts } from "$lib/stores/toasts.svelte";

  let { record }: { record: DeviceRecord } = $props();

  // Attributes shown specially are left out of the generic list.
  const special = new Set(["manage_url", "recovery_steps"]);
  const details = $derived(
    Object.entries(record.identity.attributes ?? {}).filter(([name]) => !special.has(name)),
  );
  const manageUrl = $derived(record.identity.attributes?.manage_url ?? null);
  const recoverySteps = $derived(
    (record.identity.attributes?.recovery_steps ?? "").split("\n").filter((step) => step.trim()),
  );

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
      toasts.success(next ? `Labelled "${next}".` : "Label cleared.");
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

  const rows = $derived([
    ["Family", record.key.family],
    ["Serial", record.key.serial],
    ["Model", record.identity.model],
    ["Name reported", record.identity.name ?? "—"],
    ["Mode", record.identity.mode],
    ["Link", linkText(record.link_kind, devices.nameOf)],
    ["Link id", record.identity.link],
    ["Address", record.identity.address],
    ["First seen", clockTime(record.first_seen_ms)],
    ["Last seen", `${clockTime(record.last_seen_ms)} (${timeAgo(record.last_seen_ms, clock.now)})`],
  ]);
</script>

<div class="flex flex-col gap-4">
  <label class="flex flex-col gap-1">
    <span class="micro-label">Label</span>
    <div class="flex gap-2">
      <input
        class="field flex-1"
        bind:value={label}
        placeholder={record.identity.name ?? "Give this device a name"}
        onkeydown={(e) => {
          if (e.key === "Enter") void saveLabel();
          if (e.key === "Escape") label = record.label ?? "";
        }}
        onblur={saveLabel}
        disabled={savingLabel}
      />
    </div>
    <span class="text-[0.65rem] text-surface-500">Stored in Atlas; Enter or leaving the field saves it.</span>
  </label>

  <label class="flex flex-col gap-1">
    <span class="micro-label">Robot</span>
    <select class="field" value={record.robot ?? ""} onchange={(e) => setRobot(e.currentTarget.value)} disabled={savingRobot}>
      <option value="">No robot</option>
      {#each robots.names as name (name)}
        <option value={name}>{name}</option>
      {/each}
    </select>
  </label>

  <div>
    <p class="micro-label mb-1">Versions</p>
    <dl class="grid grid-cols-[7rem_1fr] gap-y-1 text-xs">
      {#each Object.entries(record.identity.versions) as [name, version] (name)}
        <dt class="text-surface-400">{name}</dt>
        <dd class="font-mono text-surface-50">{version}</dd>
      {/each}
    </dl>
  </div>

  {#if recoverySteps.length > 0}
    <div class="rounded-base border border-warning-600/40 bg-warning-500/5 px-3 py-2">
      <p class="micro-label mb-1">Recovery steps</p>
      <ol class="list-decimal space-y-0.5 pl-4 text-xs text-surface-100">
        {#each recoverySteps as step, index (index)}
          <li>{step}</li>
        {/each}
      </ol>
    </div>
  {/if}

  {#if details.length > 0 || manageUrl}
    <div>
      <p class="micro-label mb-1">Details</p>
      <dl class="grid grid-cols-[7rem_1fr] gap-y-1 text-xs">
        {#each details as [name, value] (name)}
          <dt class="text-surface-400">{name.replaceAll("_", " ")}</dt>
          <dd class="break-all font-mono text-surface-100">{value}</dd>
        {/each}
        {#if manageUrl}
          <dt class="text-surface-400">web UI</dt>
          <dd>
            <button type="button" class="anchor break-all text-left text-xs" onclick={() => openExternal(manageUrl)}>
              {manageUrl} <i class="fa-solid fa-arrow-up-right-from-square text-[0.6rem]" aria-hidden="true"></i>
            </button>
          </dd>
        {/if}
      </dl>
    </div>
  {/if}

  <div>
    <p class="micro-label mb-1">Identity</p>
    <dl class="grid grid-cols-[7rem_1fr] gap-y-1 text-xs">
      {#each rows as [name, value] (name)}
        <dt class="text-surface-400">{name}</dt>
        <dd class="break-all text-surface-100">{value}</dd>
      {/each}
    </dl>
  </div>

  <div>
    <p class="micro-label mb-1">Capabilities</p>
    <p class="text-xs text-surface-300">{record.capabilities.join(", ")}</p>
  </div>

  {#if record.presence === "offline"}
    <div class="border-t border-surface-800 pt-3">
      <p class="mb-2 text-xs text-surface-400">
        Forgetting removes this device, its label, and its robot assignment. It comes back as new if it is seen again.
      </p>
      <ConfirmButton action={() => api.forgetDevice(record.key)} prompt="Forget this device?" confirmLabel="Forget">
        <i class="fa-solid fa-trash" aria-hidden="true"></i>Forget device
      </ConfirmButton>
    </div>
  {/if}
</div>
