<script lang="ts">
  import { api, keyString, type RobotProfile, type RobotStatus } from "$lib/api/client";
  import AsyncButton from "$lib/components/common/AsyncButton.svelte";
  import ConfirmButton from "$lib/components/common/ConfirmButton.svelte";
  import Tag from "$lib/components/common/Tag.svelte";
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
</script>

<article class="flex flex-col rounded-container border bg-surface-900/40 {editing ? 'border-primary-500/60' : 'border-surface-800'}">
  <header class="flex items-start justify-between gap-2 border-b border-surface-800 px-4 py-3">
    <div class="min-w-0">
      <h2 class="truncate text-sm font-semibold text-surface-50">{profile.name}</h2>
      {#if profile.notes}<p class="truncate text-[0.7rem] text-surface-400">{profile.notes}</p>{/if}
    </div>
    {#if badge}<Tag tone={badge.tone} icon={badge.icon} label={badge.label} />{/if}
  </header>

  <ul class="flex-1 divide-y divide-surface-800/70 px-4 py-1">
    {#each status?.roles ?? [] as role, i (i)}
      <li class="grid grid-cols-[6.5rem_1fr_auto] items-center gap-2 py-1.5 text-xs">
        <span class="truncate text-surface-400" title={role.role}>{role.role}</span>
        {#if role.device}
          <button type="button" class="truncate text-left text-surface-100 hover:underline" onclick={() => ui.openDevice(keyString(role.device!))}>
            <span class="mr-1 inline-block h-1.5 w-1.5 rounded-full {role.presence === 'online' ? 'bg-success-500' : 'bg-error-500'}" title={role.presence ?? "never seen"}></span>
            {role.device_name ?? keyString(role.device)}
          </button>
        {:else}
          <span class="text-surface-500 italic">unassigned · {role.family}</span>
        {/if}
        <span class="whitespace-nowrap font-mono text-[0.68rem]">
          {#if role.up_to_date}
            <span class="text-success-400">{role.version} <i class="fa-solid fa-check" aria-label="up to date"></i></span>
          {:else if role.version && role.target}
            <span class="text-warning-300">{role.version}</span><span class="text-surface-500"> → {role.target}</span>
          {:else}
            <span class="text-surface-500">{role.version ?? "—"}{role.target ? ` / ${role.target}` : ""}</span>
          {/if}
        </span>
      </li>
    {:else}
      <li class="py-2 text-xs text-surface-500">No roles yet. Edit the robot to add some.</li>
    {/each}
  </ul>

  {#if status && status.unassigned_devices.length > 0}
    <p class="border-t border-surface-800 px-4 py-2 text-[0.7rem] text-surface-400">
      Tagged but not in a role:
      {#each status.unassigned_devices as key, i (keyString(key))}
        <button type="button" class="text-surface-200 underline-offset-2 hover:underline" onclick={() => ui.openDevice(keyString(key))}>{devices.nameOf(key)}</button>{i < status.unassigned_devices.length - 1 ? ", " : ""}
      {/each}
    </p>
  {/if}

  <footer class="flex flex-wrap items-center gap-2 border-t border-surface-800 px-4 py-2">
    {#if status && status.state !== "ready"}
      <AsyncButton class="btn btn-sm preset-filled-primary-500" action={makeReady}>
        <i class="fa-solid fa-wand-magic-sparkles" aria-hidden="true"></i>Make ready
      </AsyncButton>
    {/if}
    <button type="button" class="btn btn-sm preset-tonal" onclick={() => ui.openRobot(profile.name)}>
      <i class="fa-solid fa-pen" aria-hidden="true"></i>Edit
    </button>
    <span class="ml-auto">
      <ConfirmButton action={remove} prompt="Delete {profile.name}?" confirmLabel="Delete" class="btn btn-sm preset-tonal text-error-300">
        <i class="fa-solid fa-trash" aria-hidden="true"></i><span class="sr-only">Delete {profile.name}</span>
      </ConfirmButton>
    </span>
  </footer>
</article>
