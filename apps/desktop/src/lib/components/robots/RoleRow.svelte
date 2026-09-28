<script lang="ts">
  import { keyString, type RobotRole } from "$lib/api/client";
  import { deviceName } from "$lib/format";
  import { devices } from "$lib/stores/devices.svelte";

  let {
    role = $bindable(),
    families,
    index,
    onremove,
  }: { role: RobotRole; families: string[]; index: number; onremove: () => void } = $props();

  const candidates = $derived(devices.all.filter((d) => d.key.family === role.family));
  const deviceValue = $derived(role.device ? keyString(role.device) : "");

  function setFamily(family: string) {
    role.family = family;
    if (role.device && role.device.family !== family) role.device = null;
  }

  function setDevice(value: string) {
    role.device = value ? (devices.get(value)?.key ?? null) : null;
  }
</script>

<li class="grid grid-cols-[1fr_7rem_auto] gap-2 rounded-base border border-surface-800 p-2">
  <label class="flex flex-col gap-0.5">
    <span class="micro-label">Role</span>
    <input class="field" bind:value={role.role} placeholder="e.g. front camera" aria-label="Role {index + 1} name" />
  </label>
  <label class="flex flex-col gap-0.5">
    <span class="micro-label">Family</span>
    <select class="field" value={role.family} onchange={(e) => setFamily(e.currentTarget.value)}>
      {#each families as family (family)}
        <option value={family}>{family}</option>
      {/each}
    </select>
  </label>
  <button type="button" class="mt-4 self-center text-surface-400 hover:text-error-300" onclick={onremove} aria-label="Remove role {role.role || index + 1}">
    <i class="fa-solid fa-xmark" aria-hidden="true"></i>
  </button>
  <label class="col-span-3 flex flex-col gap-0.5">
    <span class="micro-label">Device</span>
    <select class="field" value={deviceValue} onchange={(e) => setDevice(e.currentTarget.value)}>
      <option value="">Not assigned</option>
      {#each candidates as d (keyString(d.key))}
        <option value={keyString(d.key)}>
          {deviceName(d)} · {d.key.serial}{d.presence === "online" ? "" : " (offline)"}{d.robot && d.robot !== "" ? ` · ${d.robot}` : ""}
        </option>
      {/each}
    </select>
  </label>
</li>
