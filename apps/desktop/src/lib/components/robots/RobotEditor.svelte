<script lang="ts">
  import { api, errorText, type RobotProfile } from "$lib/api/client";
  import ConfirmButton from "$lib/components/common/ConfirmButton.svelte";
  import { sentence } from "$lib/format";
  import { devices } from "$lib/stores/devices.svelte";
  import { releases } from "$lib/stores/releases.svelte";
  import { robots } from "$lib/stores/robots.svelte";
  import { toasts } from "$lib/stores/toasts.svelte";
  import { ui } from "$lib/stores/ui.svelte";
  import RoleRow from "./RoleRow.svelte";

  let { name }: { name: string | null } = $props();

  // svelte-ignore state_referenced_locally
  const original = name ? robots.profile(name) : undefined;
  let draft = $state<RobotProfile>(
    original ? structuredClone($state.snapshot(original)) : { name: "", roles: [], targets: {}, notes: null },
  );
  let notes = $state(draft.notes ?? "");
  let saving = $state(false);
  let error = $state<string | null>(null);

  const knownFamilies = $derived(
    [...new Set([...devices.families, ...releases.families, ...draft.roles.map((r) => r.family), ...Object.keys(draft.targets)])].sort(),
  );
  const targetFamilies = $derived([...new Set([...draft.roles.map((r) => r.family), ...Object.keys(draft.targets)])].sort());

  function addRole() {
    draft.roles.push({ role: "", family: knownFamilies[0] ?? "", device: null });
  }

  function setTarget(family: string, value: string) {
    if (value.trim()) draft.targets[family] = value.trim();
    else delete draft.targets[family];
  }

  async function save(event: SubmitEvent) {
    event.preventDefault();
    if (saving) return;
    saving = true;
    error = null;
    const profile: RobotProfile = { ...$state.snapshot(draft), name: draft.name.trim(), notes: notes.trim() || null };
    try {
      await api.saveRobot(profile, original ? original.name : null);
      toasts.success(`Saved ${profile.name}.`);
      await robots.load();
      ui.openRobot(profile.name);
    } catch (e) {
      error = sentence(errorText(e));
    } finally {
      saving = false;
    }
  }

  async function remove() {
    if (!original) return;
    await api.deleteRobot(original.name);
    toasts.success(`Deleted ${original.name}.`);
    ui.close();
  }
</script>

<form class="flex flex-col gap-4 p-4" onsubmit={save}>
  <h2 class="text-base font-semibold text-surface-50">{original ? `Edit ${original.name}` : "New robot"}</h2>

  <label class="flex flex-col gap-1">
    <span class="micro-label">Name</span>
    <input class="field" bind:value={draft.name} required placeholder="e.g. Atlas-02" />
  </label>

  <label class="flex flex-col gap-1">
    <span class="micro-label">Notes</span>
    <textarea class="field min-h-14" bind:value={notes} placeholder="Optional"></textarea>
  </label>

  <section class="flex flex-col gap-2">
    <div class="flex items-center justify-between">
      <p class="micro-label">Roles</p>
      <button type="button" class="btn btn-sm preset-tonal" onclick={addRole} disabled={knownFamilies.length === 0}>
        <i class="fa-solid fa-plus" aria-hidden="true"></i>Add role
      </button>
    </div>
    {#if knownFamilies.length === 0}
      <p class="text-xs text-surface-400">Connect a device or add a release first so Atlas knows which families exist.</p>
    {/if}
    <ul class="flex flex-col gap-2">
      {#each draft.roles as _, i (i)}
        <RoleRow bind:role={draft.roles[i]} families={knownFamilies} index={i} onremove={() => draft.roles.splice(i, 1)} />
      {/each}
    </ul>
  </section>

  <section class="flex flex-col gap-2">
    <p class="micro-label">Target versions</p>
    {#if targetFamilies.length === 0}
      <p class="text-xs text-surface-400">Add a role to set the version its family should run.</p>
    {/if}
    {#each targetFamilies as family (family)}
      <label class="grid grid-cols-[7rem_1fr] items-center gap-2">
        <span class="text-xs text-surface-300">{family}</span>
        <input
          class="field font-mono"
          list="targets-{family}"
          value={draft.targets[family] ?? ""}
          onchange={(e) => setTarget(family, e.currentTarget.value)}
          placeholder="no target"
        />
        <datalist id="targets-{family}">
          {#each releases.forFamily(family) as entry (entry.id)}
            <option value={entry.version}>{entry.channel}</option>
          {/each}
        </datalist>
      </label>
    {/each}
  </section>

  {#if error}
    <p class="rounded-base border border-error-500/50 bg-error-500/10 px-3 py-2 text-xs text-error-200" role="alert">{error}</p>
  {/if}

  <div class="flex items-center gap-2 border-t border-surface-800 pt-3">
    <button type="submit" class="btn btn-sm preset-filled-primary-500" disabled={saving}>
      {#if saving}<i class="fa-solid fa-circle-notch fa-spin" aria-hidden="true"></i>{/if}Save robot
    </button>
    <button type="button" class="btn btn-sm preset-tonal" onclick={() => ui.close()}>Cancel</button>
    {#if original}
      <span class="ml-auto">
        <ConfirmButton action={remove} prompt="Delete {original.name}?" confirmLabel="Delete">Delete</ConfirmButton>
      </span>
    {/if}
  </div>
</form>
