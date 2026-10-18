<script lang="ts">
  // The board's last boots, from its event log and status: which slot runs
  // which version and what the other keeps (its update state), a strip of
  // the last 30 boots, and the newest few as rows: the boot's number, how it
  // began (a restart, or power-on after power was lost), how the boot before
  // it ended (clean, unclean, a kernel panic), a rollback in it, and the
  // kernel log it found of the boot before (pstore), opened on demand.
  import { keyString, type DeviceRecord, type DeviceStatus, type KernelRecord } from "#lib/api/client.ts";
  import Button from "#lib/components/common/Button.svelte";
  import { timeAgo } from "#lib/format.ts";
  import { deviceStatus } from "#lib/stores/status.svelte.ts";
  import KernelLogDialog from "./KernelLogDialog.svelte";

  let { record, status }: { record: DeviceRecord; status: DeviceStatus | undefined } = $props();

  const SHOWN = 30;
  const ROWS = 5;
  const history = $derived(deviceStatus.history.get(keyString(record.key)) ?? []);
  const records = $derived(new Map<string, KernelRecord>((status?.pstore ?? []).map((r) => [r.boot, r])));
  const rollbacks = $derived(
    new Map(history.filter((e) => e.origin === "board" && e.kind === "update.rolled-back" && e.boot_id).map((e) => [e.boot_id!, e.data.reason || e.message])),
  );

  type Kind = "clean" | "unclean" | "panic" | "rollback" | "slot" | "unknown";
  // Oldest first, the newest at the right.
  const boots = $derived(
    history
      .filter((e) => e.origin === "board" && e.kind === "boot")
      .sort((a, b) => a.at_ms - b.at_ms)
      .slice(-SHOWN)
      .map((e, i, all) => {
        const id = e.boot_id ?? "";
        const clean = e.data.previous_clean;
        // An older board's events don't say how the boot began: this boot's status does.
        const reset = e.data.reset ?? (id && id === status?.boot?.id ? status?.boot?.reset : undefined);
        const log = records.get(id);
        const rollback = rollbacks.get(id);
        const moved = i > 0 && !!e.data.slot && !!all[i - 1].data.slot && e.data.slot !== all[i - 1].data.slot;
        const kind: Kind = log?.panic
          ? "panic"
          : clean === "false"
            ? "unclean"
            : rollback
              ? "rollback"
              : moved
                ? "slot"
                : clean === "true"
                  ? "clean"
                  : "unknown";
        const began = reset === "power-on" ? "power-on" : reset === "reset" ? "restart" : "—";
        const ended =
          kind === "panic" ? "panic" : clean === "false" ? (reset === "power-on" ? "power lost" : "unclean") : clean === "true" ? "clean" : "unknown";
        return {
          id,
          at: e.at_ms,
          count: e.data.count ?? null,
          slot: e.data.slot || null,
          kind,
          began,
          ended,
          rollback,
          log,
          title: `${new Date(e.at_ms).toLocaleString()} · ${e.message}`,
        };
      }),
  );
  const rows = $derived(boots.slice(-ROWS).reverse());
  const unclean = $derived(boots.filter((b) => b.kind === "unclean" || b.kind === "panic").length);
  const count = $derived(status?.boot?.count ?? (Number(boots.at(-1)?.count) || null));
  const update = $derived(status?.update ?? null);
  const other = $derived(update?.slot_active === "A" ? "B" : update?.slot_active === "B" ? "A" : null);

  let open = $state<{ boot: string; title: string } | null>(null);
</script>

{#if update || boots.length > 0}
  <section class="panel" aria-label="Last boots">
    <div class="panel-head">
      <h2 class="panel-title">Last boots</h2>
      {#if boots.length > 0}
        <span class="panel-note" class:warn={unclean > 0}>{#if count}boot {count} · {/if}{unclean} unclean of {boots.length}</span>
      {/if}
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
      <ol class="rows" aria-label="Newest boots">
        {#each rows as b (b.at)}
          <li title={b.title}>
            <span class="num mono">{b.count ? `#${b.count}` : "—"}</span>
            <span class="began" class:power={b.began === "power-on"}>{b.began}</span>
            <span class="ended"><span class="swatch {b.kind === 'rollback' || b.kind === 'slot' ? 'clean' : b.kind}"></span>{b.ended}</span>
            <span class="where">
              {#if b.rollback}<span class="tag" title="Rolled back: {b.rollback}">rollback</span>{:else if b.slot}<span title="Slot {b.slot}">slot {b.slot}</span>{/if}
            </span>
            <span class="ago">{timeAgo(b.at)}</span>
            <span class="log">
              {#if b.log}
                <Button
                  size="sm"
                  variant={b.log.panic ? "tint" : "ghost"}
                  icon="file-text"
                  label="Kernel log of the boot before {b.count ? `#${b.count}` : 'this one'}"
                  title="What the kernel printed until the boot before ended{b.log.panic ? ' (a panic record)' : ''}"
                  onclick={() => (open = { boot: b.id, title: `Kernel log before boot ${b.count ? `#${b.count}` : ""}${b.log?.panic ? " · panic" : ""}` })}
                />
              {/if}
            </span>
          </li>
        {/each}
      </ol>
    {/if}
  </section>
{/if}

{#if open}
  <KernelLogDialog deviceKey={record.key} boot={open.boot} title={open.title} onclose={() => (open = null)} />
{/if}

<style>
  .panel-note.warn {
    color: var(--warn-fg);
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
    height: 12px;
    border-radius: 1px;
  }
  .rows {
    display: flex;
    flex-direction: column;
    margin: 0;
    padding: 0;
    list-style: none;
    font-size: 11.5px;
  }
  .rows li {
    display: grid;
    grid-template-columns: 28px 50px minmax(0, 1fr) 50px 42px 22px;
    align-items: center;
    gap: 4px;
    min-height: 24px;
    border-top: 1px solid var(--hairline);
  }
  .num {
    color: var(--fg);
  }
  .began,
  .where,
  .ago {
    color: var(--fg-muted);
    white-space: nowrap;
  }
  .began.power {
    color: var(--warn-fg);
  }
  .ended {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    min-width: 0;
    white-space: nowrap;
    overflow: hidden;
  }
  .ago {
    text-align: right;
  }
  .tag {
    padding: 0 5px;
    font-size: 11px;
    color: var(--warn-fg);
    background: var(--warn-bg);
    border-radius: var(--r-sm);
  }
  .log {
    display: flex;
    justify-content: flex-end;
  }
  .swatch {
    display: inline-block;
    flex: none;
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
  .panic {
    background: var(--err);
  }
  .rollback {
    background: repeating-linear-gradient(135deg, var(--warn) 0 2px, var(--series-1) 2px 4px);
  }
  .unknown {
    background: var(--glass-border-strong);
  }
</style>
