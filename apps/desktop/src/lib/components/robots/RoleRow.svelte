<script lang="ts">
  import { keyString, type RobotRole } from "$lib/api/client";
  import Button from "$lib/components/common/Button.svelte";
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

<div class="glass flex flex-col gap-2 p-3">
  <div class="grid grid-cols-[1fr_8rem_auto] items-center gap-2">
    <input class="input" bind:value={role.role} placeholder="Role, e.g. front camera" aria-label="Role {index + 1} name" />
    <select class="select" value={role.family} onchange={(e) => setFamily(e.currentTarget.value)} aria-label="Role {index + 1} family">
      {#each families as family (family)}
        <option value={family}>{family}</option>
      {/each}
    </select>
    <Button variant="ghost" size="sm" icon="x" label="Remove role {role.role || index + 1}" onclick={onremove} />
  </div>
  <select class="select" value={deviceValue} onchange={(e) => setDevice(e.currentTarget.value)} aria-label="Role {index + 1} device">
    <option value="">No device yet</option>
    {#each candidates as d (keyString(d.key))}
      <option value={keyString(d.key)}>
        {deviceName(d)} · {d.key.serial}{d.presence === "online" ? "" : " (offline)"}{d.robot ? ` · ${d.robot}` : ""}
      </option>
    {/each}
  </select>
</div>
