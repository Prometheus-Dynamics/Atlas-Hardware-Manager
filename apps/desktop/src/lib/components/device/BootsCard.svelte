<script lang="ts">
  // The board's storage and its recent boots: which slot runs which version
  // and what the other keeps (from its update state), then the last boots
  // from its event log as a strip: clean, after a crash or power cut, or
  // into another slot (an update or a rollback).
  import { keyString, type DeviceRecord, type DeviceStatus } from "#lib/api/client.ts";
  import { storageName } from "#lib/present.ts";
  import { deviceStatus } from "#lib/stores/status.svelte.ts";

  let { record, status }: { record: DeviceRecord; status: DeviceStatus | undefined } = $props();

  const SHOWN = 30;
  const history = $derived(deviceStatus.history.get(keyString(record.key)) ?? []);
  // Oldest first, the newest at the right.
  const boots = $derived(
    history
      .filter((e) => e.origin === "board" && e.kind === "boot")
      .sort((a, b) => a.at_ms - b.at_ms)
      .slice(-SHOWN)
      .map((e, i, all) => {
        const clean = e.data.previous_clean;
        const moved = i > 0 && !!e.data.slot && !!all[i - 1].data.slot && e.data.slot !== all[i - 1].data.slot;
        const kind = clean === "false" ? "unclean" : moved ? "slot" : clean === "true" ? "clean" : "unknown";
        return { at: e.at_ms, kind, title: `${new Date(e.at_ms).toLocaleString()} · ${e.message}` };
      }),
  );
  const unclean = $derived(boots.filter((b) => b.kind === "unclean").length);
  const update = $derived(status?.update ?? null);
  const storage = $derived(storageName(record.identity.attributes?.storage) ?? "Storage");
  const other = $derived(update?.slot_active === "A" ? "B" : update?.slot_active === "B" ? "A" : null);
</script>

{#if update || boots.length > 0}
  <section class="panel" aria-label="Storage and boots">
    <div class="panel-head">
      <h2 class="panel-title">{storage} and boots</h2>
      {#if boots.length > 0}<span class="panel-note" class:warn={unclean > 0}>{unclean} unclean of the last {boots.length}</span>{/if}
    </div>
    {#if update?.slot_active}
      <dl class="slots">
        <div>
          <dt><span class="swatch running"></span>Slot {update.slot_active}</dt>
          <dd><span class="mono">{update.version_active ?? "—"}</span> running</dd>
        </div>
        {#if other}
          <div>
            <dt><span class="swatch kept"></span>Slot {other}</dt>
            <dd>
              {#if update.version_staged && update.slot_staged === other}<span class="mono">{update.version_staged}</span> staged
              {:else if update.version_previous}<span class="mono">{update.version_previous}</span> kept to go back to
              {:else}nothing kept{/if}
            </dd>
          </div>
        {/if}
      </dl>
    {/if}
    {#if boots.length > 0}
      <div class="strip" role="img" aria-label="Last {boots.length} boots: {unclean} unclean">
        {#each boots as b (b.at)}<span class="boot {b.kind}" title={b.title}></span>{/each}
      </div>
      <div class="legend">
        <span><span class="swatch clean"></span>clean</span>
        <span><span class="swatch unclean"></span>after a crash or power cut</span>
        <span><span class="swatch slot"></span>into the other slot</span>
      </div>
    {/if}
  </section>
{/if}

<style>
  .panel-note.warn {
    color: var(--warn-fg);
  }
  .legend {
    font-size: 11.5px;
    color: var(--fg-muted);
  }
  .slots {
    display: grid;
    grid-template-columns: repeat(2, minmax(0, 1fr));
    gap: 8px;
    font-size: 12.5px;
  }
  dt {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: 11.5px;
    color: var(--fg-muted);
  }
  .mono {
    font-family: var(--font-code);
    font-size: 12px;
  }
  .strip {
    display: grid;
    grid-template-columns: repeat(30, minmax(0, 1fr));
    gap: 2px;
  }
  .boot {
    height: 16px;
    border-radius: 1px;
  }
  .legend {
    display: flex;
    flex-wrap: wrap;
    gap: 4px 12px;
  }
  .legend > span {
    display: inline-flex;
    align-items: center;
    gap: 5px;
  }
  .swatch {
    display: inline-block;
    width: 8px;
    height: 8px;
    border-radius: 1px;
  }
  .running,
  .slot {
    background: var(--series-1);
  }
  .kept {
    background: color-mix(in srgb, var(--series-1) 45%, var(--glass));
  }
  .clean {
    background: color-mix(in srgb, var(--ok) 55%, var(--glass));
  }
  .unclean {
    background: var(--warn);
  }
  .unknown {
    background: var(--glass-border-strong);
  }
</style>
