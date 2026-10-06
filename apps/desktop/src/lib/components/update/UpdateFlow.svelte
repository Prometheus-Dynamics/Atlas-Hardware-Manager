<script lang="ts">
  // The update flow for a selection or robot "Make ready": choose releases,
  // preview the plan, start. Recovery devices use the Flash tab instead.
  import { keyString, type StagedRollout, type UpdateRequestInput } from "#lib/api/client.ts";
  import Button from "#lib/components/common/Button.svelte";
  import Icon from "#lib/components/common/Icon.svelte";
  import SegmentedControl from "#lib/components/common/SegmentedControl.svelte";
  import { primaryVersion } from "#lib/format.ts";
  import { devices } from "#lib/stores/devices.svelte.ts";
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

  function currentVersions(family: string): string[] {
    return records
      .filter((r) => r?.key.family === family)
      .map((r) => (r ? primaryVersion(r.identity) : null))
      .filter((v): v is string => !!v);
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
  <section class="flex flex-col gap-2">
    <h3 class="section-title">Devices</h3>
    <ul class="flex flex-wrap gap-1.5">
      {#each draft.keys as key (keyString(key))}
        {@const record = devices.get(key)}
        {@const online = record?.presence === "online"}
        <li class="chip" class:offline={!online}>
          <span class="dot" class:on={online}></span>{devices.nameOf(key)}{online ? "" : " · offline"}
        </li>
      {/each}
    </ul>
  </section>

  <section class="flex flex-col gap-4">
    <h3 class="section-title">Choose {draft.families.length === 1 ? "a release" : "releases"}</h3>
    {#each draft.families as family (family)}
      <ReleasePicker
        {family}
        count={draft.keys.filter((k) => k.family === family).length}
        current={currentVersions(family)}
        bind:choice={draft.choices[family]}
      />
    {/each}

    <div class="flex flex-col gap-1.5">
      <span class="text-[12.5px] font-medium text-fg-muted">Staged rollout</span>
      <SegmentedControl options={stagedOptions} bind:value={draft.staged} label="Staged rollout" size="sm" />
      <p class="hint">{stagedOptions.find((o) => o.value === draft.staged)?.title}.</p>
    </div>
  </section>

  <section class="flex flex-col gap-2">
    <h3 class="section-title flex items-center gap-2">
      What will happen
      {#if draft.planning}<Icon name="loader-2" size={14} class="spin text-fg-faint" />{/if}
    </h3>
    {#if draft.missing.length > 0}
      <p class="text-[13px] text-fg-muted">Choose a version for {draft.missing.join(" and ")} to see the plan.</p>
    {:else if draft.planError}
      <p class="problem" role="alert" in:rise>
        <Icon name="alert-circle" size={16} />{draft.planError}
      </p>
      {#if draft.checksumFailed}
        <div class="flex gap-2">
          <Button size="sm" icon="refresh" disabled={draft.starting} onclick={() => start(false)}>Download again</Button>
          <Button size="sm" variant="danger" icon="alert-triangle" disabled={draft.starting} onclick={() => start(true)}>
            Flash anyway
          </Button>
        </div>
      {/if}
    {:else if draft.plan}
      <PlanTable plan={draft.plan} />
    {/if}
    {#if draft.unsigned.length > 0}
      <p class="warn"><Icon name="alert-triangle" size={16} />Unsigned release. Atlas can't verify where it came from, but won't stop you.</p>
    {/if}
  </section>

  <Button
    variant="primary"
    size="lg"
    full
    icon={recover ? "bolt" : "arrow-up"}
    busy={draft.starting}
    disabled={!draft.plan || draft.planning}
    onclick={() => start(false)}
  >
    {recover ? "Flash" : "Update"} {count} device{count === 1 ? "" : "s"}
  </Button>
</div>

<style>
  .chip {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    height: 26px;
    padding: 0 10px;
    border-radius: var(--r-pill);
    background: var(--glass);
    border: 1px solid var(--glass-border);
    font-size: 12.5px;
    color: var(--fg);
  }
  .chip.offline {
    color: var(--err-fg);
  }
  .dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--offline);
  }
  .dot.on {
    background: var(--ok);
  }
  .problem,
  .warn {
    display: flex;
    gap: 8px;
    align-items: flex-start;
    padding: 10px 14px;
    border-radius: var(--r-card);
    font-size: 13px;
    line-height: 1.45;
  }
  .problem {
    background: var(--err-bg);
    color: var(--err-fg);
  }
  .warn {
    background: var(--warn-bg);
    color: var(--warn-fg);
  }
</style>
