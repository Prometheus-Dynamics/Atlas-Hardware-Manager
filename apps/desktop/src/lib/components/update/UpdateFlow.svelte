<script lang="ts">
  // The update flow for a selection or a robot's "Make ready": choose a
  // release per family, see what will happen, start. One device's own
  // installs live in its Software tab.
  import type { ReleaseEntry, StagedRollout, UpdateRequestInput } from "#lib/api/client.ts";
  import Button from "#lib/components/common/Button.svelte";
  import Icon from "#lib/components/common/Icon.svelte";
  import SegmentedControl from "#lib/components/common/SegmentedControl.svelte";
  import Disclosure from "#lib/components/device/Disclosure.svelte";
  import { imagesFor, newestFirst, updateMethods } from "#lib/components/software/software.ts";
  import { primaryVersion } from "#lib/format.ts";
  import { isRecovery } from "#lib/present.ts";
  import { devices } from "#lib/stores/devices.svelte.ts";
  import { releases } from "#lib/stores/releases.svelte.ts";
  import { rise } from "#lib/ui/motion.ts";
  import PlanTable from "./PlanTable.svelte";
  import ReleasePicker from "./ReleasePicker.svelte";
  import { UpdateDraft } from "./updateDraft.svelte";

  let { request, onstarted }: { request: UpdateRequestInput; onstarted?: (job: number) => void } = $props();

  // The request is the starting point; edits here stay local to the flow.
  // svelte-ignore state_referenced_locally
  const draft = new UpdateDraft(request);
  const records = $derived(draft.keys.map((k) => devices.get(k)));
  const recover = $derived(records.length > 0 && records.every((r) => r?.identity.mode === "recovery"));
  const count = $derived(draft.plan ? draft.plan.devices.length : draft.keys.length);

  const names = $derived(draft.keys.map((k) => devices.nameOf(k)));
  const offline = $derived(records.filter((r) => r?.presence !== "online").length);
  const who = $derived(
    names.length === 1
      ? names[0]
      : `${names.length} devices: ${names.slice(0, 2).join(", ")}${names.length > 2 ? `, +${names.length - 2}` : ""}`,
  );

  function currentVersions(family: string): string[] {
    return records
      .filter((r) => r?.key.family === family)
      .map((r) => (r ? primaryVersion(r.identity) : null))
      .filter((v): v is string => !!v);
  }

  /** Images for a family's devices, including ones made for the same board under another family. */
  function imagesForFamily(family: string): ReleaseEntry[] {
    const found = records.flatMap((r) => (r?.key.family === family ? imagesFor(r, releases.entries) : []));
    const unique = new Map([...releases.forFamily(family), ...found].map((e) => [e.id, e]));
    return [...unique.values()].sort(newestFirst);
  }

  /** Devices that install a file Atlas sends, so a typed version can't work. */
  function needsFile(family: string): boolean {
    return records.some((r) => r?.key.family === family && (isRecovery(r) || updateMethods(r).length > 0));
  }

  async function start(ignoreChecksum = false) {
    const job = await draft.start(ignoreChecksum);
    if (job !== null) onstarted?.(job);
  }

  const stagedOptions: { value: StagedRollout; label: string; title: string }[] = [
    { value: "auto", label: "Auto", title: "One device first when a family has three or more" },
    { value: "on", label: "One first", title: "One device per family first; the rest wait for it to verify" },
    { value: "off", label: "All at once", title: "All devices at once, within resource limits" },
  ];
</script>

<div class="flex flex-col gap-6">
  <p class="flex items-center gap-2 text-[13px] text-fg-muted">
    <Icon name="cpu" size={15} class="shrink-0 text-fg-faint" />
    <span class="min-w-0 truncate" title={names.join(", ")}>{who}</span>
    {#if offline > 0}<span class="shrink-0 text-err-fg">· {offline} offline</span>{/if}
  </p>

  <section class="flex flex-col gap-4">
    <h3 class="section-title">Choose {draft.families.length === 1 ? "a release" : "releases"}</h3>
    {#each draft.families as family (family)}
      <ReleasePicker
        {family}
        count={draft.keys.filter((k) => k.family === family).length}
        current={currentVersions(family)}
        needsFile={needsFile(family)}
        entries={imagesForFamily(family)}
        bind:choice={draft.choices[family]}
      />
    {/each}
  </section>

  {#if draft.keys.length >= 2}
    <Disclosure title="Options">
      <div class="flex flex-col gap-1.5">
        <span class="text-[12.5px] font-medium text-fg-muted">Staged rollout</span>
        <SegmentedControl options={stagedOptions} bind:value={draft.staged} label="Staged rollout" size="sm" />
        <p class="hint">{stagedOptions.find((o) => o.value === draft.staged)?.title}.</p>
      </div>
    </Disclosure>
  {/if}

  <section class="flex flex-col gap-1">
    <h3 class="section-title flex items-center gap-2">
      What will happen
      {#if draft.planning}<Icon name="loader-2" size={14} class="spin text-fg-faint" />{/if}
    </h3>
    {#if draft.missing.length > 0}
      <p class="text-[13px] text-fg-muted">Choose a version for {draft.missing.join(" and ")} to see the plan.</p>
    {:else if draft.plan}
      <PlanTable plan={draft.plan} />
    {/if}
    {#if draft.unsigned.length > 0}
      <p class="flex items-center gap-1.5 text-[12px] text-warn-fg">
        <Icon name="alert-triangle" size={14} />Unsigned: Atlas can't tell where it came from.
      </p>
    {/if}
  </section>

  <div class="flex flex-col gap-2">
    {#if draft.planError && draft.missing.length === 0}
      <p class="problem" role="alert" in:rise><Icon name="alert-circle" size={16} />{draft.planError}</p>
      {#if draft.checksumFailed}
        <div class="flex gap-2">
          <Button size="sm" icon="refresh" disabled={draft.starting} onclick={() => start(false)}>Download again</Button>
          <Button size="sm" variant="danger" icon="alert-triangle" disabled={draft.starting} onclick={() => start(true)}>
            Use it anyway
          </Button>
        </div>
      {/if}
    {/if}
    <Button
      variant="primary"
      size="lg"
      full
      icon={recover ? "bolt" : "arrow-up"}
      busy={draft.starting}
      disabled={!draft.plan || draft.planning}
      onclick={() => start(false)}
    >
      {recover ? "Install on" : "Update"} {count} device{count === 1 ? "" : "s"}
    </Button>
  </div>
</div>

<style>
  .problem {
    display: flex;
    gap: 8px;
    align-items: flex-start;
    padding: 10px 14px;
    border-radius: var(--r-card);
    background: var(--err-bg);
    color: var(--err-fg);
    font-size: 13px;
    line-height: 1.45;
  }
</style>
