<script lang="ts">
  import { api, errorText, type RobotProfile } from "#lib/api/client.ts";
  import Button from "#lib/components/common/Button.svelte";
  import ConfirmButton from "#lib/components/common/ConfirmButton.svelte";
  import Field from "#lib/components/common/Field.svelte";
  import Icon from "#lib/components/common/Icon.svelte";
  import { rise, softFade } from "#lib/ui/motion.ts";
  import { sentence } from "#lib/format.ts";
  import { devices } from "#lib/stores/devices.svelte.ts";
  import { releases } from "#lib/stores/releases.svelte.ts";
  import { robots } from "#lib/stores/robots.svelte.ts";
  import { toasts } from "#lib/stores/toasts.svelte.ts";
  import { ui } from "#lib/stores/ui.svelte.ts";
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

<form class="flex flex-col gap-6 px-6 pb-6 pt-5" onsubmit={save}>
  <h2 class="pr-10 text-[18px] font-semibold text-fg">{original ? `Edit ${original.name}` : "New robot"}</h2>

  <Field label="Name">
    <input class="input" bind:value={draft.name} required placeholder="e.g. Atlas-02" />
  </Field>

  <Field label="Notes">
    <textarea class="textarea" bind:value={notes} placeholder="Optional"></textarea>
  </Field>

  <section class="flex flex-col gap-2.5">
    <div class="flex items-center justify-between">
      <h3 class="section-title">Roles</h3>
      <Button size="sm" icon="plus" onclick={addRole} disabled={knownFamilies.length === 0}>Add role</Button>
    </div>
    {#if knownFamilies.length === 0}
      <p class="text-[13px] text-fg-muted">Connect a device or add a release first, so there are device families to pick from.</p>
    {/if}
    <div class="flex flex-col gap-2">
      {#each draft.roles as _, i (i)}
        <div in:rise out:softFade>
          <RoleRow bind:role={draft.roles[i]} families={knownFamilies} index={i} onremove={() => draft.roles.splice(i, 1)} />
        </div>
      {/each}
    </div>
  </section>

  <section class="flex flex-col gap-2.5">
    <h3 class="section-title">Versions to run</h3>
    {#if targetFamilies.length === 0}
      <p class="text-[13px] text-fg-muted">Add a role to set the version its family should run.</p>
    {/if}
    {#each targetFamilies as family (family)}
      <label class="grid grid-cols-[8rem_1fr] items-center gap-3">
        <span class="truncate text-[13px] text-fg-muted">{family}</span>
        <input
          class="input mono"
          list="targets-{family}"
          value={draft.targets[family] ?? ""}
          onchange={(e) => setTarget(family, e.currentTarget.value)}
          placeholder="No target"
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
    <p class="flex items-start gap-2 rounded-[var(--r-card)] bg-[var(--err-bg)] px-3.5 py-2.5 text-[13px] text-err-fg" role="alert">
      <Icon name="alert-circle" size={16} />{error}
    </p>
  {/if}

  <div class="flex items-center gap-2 border-t border-hairline pt-4">
    <Button type="submit" variant="primary" busy={saving}>Save robot</Button>
    <Button variant="ghost" onclick={() => ui.close()}>Cancel</Button>
    {#if original}
      <span class="ml-auto">
        <ConfirmButton action={remove} icon="trash" prompt="Delete {original.name}?" confirmLabel="Delete">Delete</ConfirmButton>
      </span>
    {/if}
  </div>
</form>
