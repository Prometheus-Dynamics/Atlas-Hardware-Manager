<script lang="ts">
  import { goto } from "$app/navigation";
  import { api, keyString, type RobotProfile, type RobotStatus } from "$lib/api/client";
  import Button from "$lib/components/common/Button.svelte";
  import ConfirmButton from "$lib/components/common/ConfirmButton.svelte";
  import Icon from "$lib/components/common/Icon.svelte";
  import IconTile from "$lib/components/common/IconTile.svelte";
  import Pill from "$lib/components/common/Pill.svelte";
  import StatusDot from "$lib/components/common/StatusDot.svelte";
  import { devices } from "$lib/stores/devices.svelte";
  import { system } from "$lib/stores/system.svelte";
  import { toasts } from "$lib/stores/toasts.svelte";
  import { ui } from "$lib/stores/ui.svelte";
  import { ROBOT_STATE } from "./robotState";

  let { profile, status }: { profile: RobotProfile; status: RobotStatus | undefined } = $props();

  const badge = $derived(status ? ROBOT_STATE[status.state] : null);
  const editing = $derived(ui.panel?.kind === "robot" && ui.panel.name === profile.name);

  async function makeReady() {
    const request = await api.robotUpdateRequest(profile.name, system.settings?.staged_default ?? null);
    if (!request) {
      toasts.success(`${profile.name} is already ready.`);
      return;
    }
    ui.openUpdate(request, `Make ${profile.name} ready`);
  }

  async function remove() {
    await api.deleteRobot(profile.name);
    if (editing) ui.close();
    toasts.success(`Deleted ${profile.name}.`);
  }

  function showDevices() {
    ui.robot = profile.name;
    void goto("/");
  }
</script>

<article class="glass card flex flex-col" class:editing>
  <header class="flex items-start gap-3 px-5 pt-4">
    <IconTile icon="robot" />
    <div class="min-w-0 flex-1">
      <h2 class="truncate text-[15px] font-semibold text-fg">{profile.name}</h2>
      <p class="truncate text-[12.5px] text-fg-muted">{profile.notes ?? `${profile.roles.length} role${profile.roles.length === 1 ? "" : "s"}`}</p>
    </div>
    {#if badge}<Pill tone={badge.tone} icon={badge.icon} label={badge.label} />{/if}
  </header>

  <ul class="flex flex-1 flex-col gap-0.5 px-3 py-3">
    {#each status?.roles ?? [] as role, i (i)}
      <li class="role">
        <span class="truncate text-[12.5px] text-fg-faint" title={role.role}>{role.role[0]?.toUpperCase() + role.role.slice(1)}</span>
        {#if role.device}
          <button type="button" class="flex min-w-0 items-center gap-2 text-left text-[13px] text-fg hover:underline" onclick={() => ui.openDevice(keyString(role.device!))}>
            <StatusDot state={role.presence === "online" ? "online" : "offline"} label={role.presence ?? "Never seen"} />
            <span class="truncate">{role.device_name ?? keyString(role.device)}</span>
          </button>
        {:else}
          <span class="flex items-center gap-2 text-[13px] text-fg-faint"><Icon name="circle-dashed" size={14} />Empty · {role.family}</span>
        {/if}
        <span class="justify-self-end whitespace-nowrap text-[12px]">
          {#if role.up_to_date}
            <span class="mono inline-flex items-center gap-1 text-ok-fg">{role.version}<Icon name="check" size={13} label="Up to date" /></span>
          {:else if role.version && role.target}
            <span class="mono text-warn-fg">{role.version}</span><span class="mono text-fg-faint"> → {role.target}</span>
          {:else}
            <span class="mono text-fg-faint">{role.version ?? "—"}{role.target ? ` / ${role.target}` : ""}</span>
          {/if}
        </span>
      </li>
    {:else}
      <li class="px-2 py-2 text-[13px] text-fg-faint">No roles yet. Edit the robot to add some.</li>
    {/each}
  </ul>

  {#if status && status.unassigned_devices.length > 0}
    <p class="mx-5 mb-3 text-[12.5px] text-fg-muted">
      On this robot but not in a role:
      {#each status.unassigned_devices as key, i (keyString(key))}
        <button type="button" class="text-fg hover:underline" onclick={() => ui.openDevice(keyString(key))}>{devices.nameOf(key)}</button>{i < status.unassigned_devices.length - 1 ? ", " : ""}
      {/each}
    </p>
  {/if}

  <footer class="flex flex-wrap items-center gap-2 border-t border-hairline px-4 py-3">
    {#if status && status.state !== "ready"}
      <Button variant="tint" size="sm" icon="wand" action={makeReady}>Make ready</Button>
    {/if}
    <Button size="sm" icon="layout-grid" onclick={showDevices}>Devices</Button>
    <Button variant="ghost" size="sm" icon="pencil" onclick={() => ui.openRobot(profile.name)}>Edit</Button>
    <span class="ml-auto">
      <ConfirmButton action={remove} size="sm" variant="ghost" icon="trash" label="Delete {profile.name}" prompt="Delete {profile.name}?" confirmLabel="Delete" />
    </span>
  </footer>
</article>

<style>
  .card {
    border-radius: var(--r-panel);
    transition:
      border-color var(--t-fast),
      box-shadow var(--t-med);
  }
  .card.editing {
    border-color: var(--glass-border-strong);
    box-shadow: var(--shadow-lift);
  }
  .role {
    display: grid;
    grid-template-columns: 7rem minmax(0, 1fr) auto;
    align-items: center;
    gap: 10px;
    padding: 7px 8px;
    border-radius: 10px;
  }
  .role:hover {
    background: var(--glass);
  }
</style>
