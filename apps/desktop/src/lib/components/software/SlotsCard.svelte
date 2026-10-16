<script lang="ts">
  // A slots board's installed OS images: which runs, which is the default,
  // the way back, and the room for more. Boot one once (on trial: the board
  // goes back by itself when it doesn't come up), make one that booted well
  // the default, pin one, remove one. Removing and restarting ask first.
  import { api, errorText, type DeviceRecord, type Slot, type SlotCommand, type SlotSet } from "#lib/api/client.ts";
  import Button from "#lib/components/common/Button.svelte";
  import Icon from "#lib/components/common/Icon.svelte";
  import Pill from "#lib/components/common/Pill.svelte";
  import Skeleton from "#lib/components/common/Skeleton.svelte";
  import GuardedDialog from "#lib/components/device/GuardedDialog.svelte";
  import { bytes, deviceName, sentence, timeAgo } from "#lib/format.ts";
  import { deviceStatus } from "#lib/stores/status.svelte.ts";
  import { toasts } from "#lib/stores/toasts.svelte.ts";

  let {
    record,
    set = $bindable(null),
    reload = 0,
  }: { record: DeviceRecord; set?: SlotSet | null; /** Bump to read the slots again. */ reload?: number } = $props();

  let loading = $state(true);
  let error = $state<string | null>(null);
  let busy = $state<string | null>(null);
  let asking = $state<{ command: SlotCommand; title: string; body: string; label: string } | null>(null);

  async function refresh() {
    try {
      set = await api.deviceSlots(record.key);
      error = null;
    } catch (e) {
      error = sentence(errorText(e));
    } finally {
      loading = false;
    }
  }

  $effect(() => {
    void record.key;
    void reload;
    void refresh();
    const timer = setInterval(() => void refresh(), 15_000);
    return () => clearInterval(timer);
  });

  const name = (slot: Slot) => [pretty(slot.family), slot.version].filter(Boolean).join(" ") || slot.id;
  function pretty(family: string): string {
    if (family === "photonvision") return "PhotonVision";
    if (family === "helios") return "HeliOS";
    return family ? family[0].toUpperCase() + family.slice(1) : "";
  }
  const has = (slot: Slot, flag: string) => slot.flags.includes(flag);

  /** Why it can't be removed, as the writer would say. */
  function keep(slot: Slot): string | null {
    if (has(slot, "running")) return "It's running";
    if (has(slot, "default")) return "It's the default; make another the default first";
    if (has(slot, "trial")) return "It's on trial";
    if (slot.pinned) return "Pinned; unpin it first";
    return null;
  }

  const used = $derived(set ? set.slots.reduce((sum, s) => sum + s.size, 0) : 0);
  const room = $derived(set ? Math.max(0, set.free_bytes - set.reserve_bytes) : 0);
  const total = $derived(used + (set?.free_bytes ?? 0));

  async function run(command: SlotCommand, done: string) {
    busy = `${command.command}:${command.id}`;
    try {
      await api.slotCommand(record.key, command);
      toasts.success(done);
      void deviceStatus.refresh(record.key);
    } catch (e) {
      toasts.error(sentence(errorText(e)));
    } finally {
      busy = null;
      asking = null;
      void refresh();
    }
  }

  function bootOnce(slot: Slot) {
    asking = {
      command: { command: "boot-once", id: slot.id },
      title: `Restart ${deviceName(record)} into ${name(slot)}?`,
      body: `It restarts into ${name(slot)} once, on trial. If it comes up healthy it becomes the default; if not, the board goes back to ${set?.slot_default ? "the default" : "the running one"} by itself.`,
      label: "Restart into it",
    };
  }
  function remove(slot: Slot) {
    asking = {
      command: { command: "remove", id: slot.id },
      title: `Remove ${name(slot)} from ${deviceName(record)}?`,
      body: `Its image (${bytes(slot.size)}) is deleted from the board. Data and settings stay.`,
      label: "Remove",
    };
  }
</script>

<section class="panel" aria-label="Slots">
  <div class="panel-head">
    <h2 class="panel-title">Slots</h2>
    {#if set}
      <span class="panel-note">{set.slots.length} installed · {bytes(room)} free for more</span>
    {/if}
    <Button size="sm" variant="ghost" icon="refresh" label="Refresh" title="Read the slots again" onclick={() => void refresh()} />
  </div>

  {#if loading && !set}
    <Skeleton height={96} />
  {:else if error && !set}
    <p class="problem"><Icon name="alert-circle" size={15} />{error}</p>
  {:else if set}
    <ul class="rows">
      {#each set.slots as slot (slot.id)}
        {@const why = keep(slot)}
        <li class="row" class:current={has(slot, "running")}>
          <p class="line">
            <span class="truncate text-[13px] font-medium text-fg">{name(slot)}</span>
            {#if slot.variant}<span class="text-[12px] text-fg-faint">{slot.variant}</span>{/if}
            {#if slot.verity}<span title="Checked block by block as it's read (dm-verity)" class="flex text-fg-faint"><Icon name="shield-check" size={13} /></span>{/if}
            <span class="flags">
              {#if has(slot, "running")}<Pill tone="success" label="Running" />{/if}
              {#if has(slot, "default")}<Pill tone="primary" label="Default" />{/if}
              {#if has(slot, "trial")}<Pill tone="warning" label="On trial" />{/if}
              {#if has(slot, "previous")}<Pill label="Way back" title="The default before this one: where the board goes if the default can't start" />{/if}
              {#if slot.good === "no"}<Pill tone="error" label="Didn't come up" title="Its last trial boot failed; the board went back" />{/if}
            </span>
          </p>
          <div class="line">
            <p class="min-w-0 flex-1 truncate text-[11px] text-fg-faint" title={slot.id}><span class="mono">{slot.id}</span> · {bytes(slot.size)}{slot.last_boot ? ` · booted ${timeAgo(Date.parse(slot.last_boot))}` : slot.good === "" ? " · never booted" : ""}</p>
            <span class="acts">
              {#if !has(slot, "running")}
                <Button size="sm" icon="player-play" busy={busy === `boot-once:${slot.id}`} disabled={!!busy} onclick={() => bootOnce(slot)}>Boot once</Button>
              {/if}
              {#if !has(slot, "default") && slot.good === "yes"}
                <Button size="sm" busy={busy === `set-default:${slot.id}`} disabled={!!busy} onclick={() => void run({ command: "set-default", id: slot.id }, `${name(slot)} is now the default on ${deviceName(record)}.`)}>Make default</Button>
              {/if}
              <Button
                size="sm"
                variant="ghost"
                icon={slot.pinned ? "pinned" : "pin"}
                label={slot.pinned ? "Unpin" : "Pin"}
                title={slot.pinned ? "Pinned: kept from removal. Unpin" : "Pin: keep it from removal"}
                disabled={!!busy}
                onclick={() => void run({ command: "pin", id: slot.id, pinned: !slot.pinned }, `${name(slot)} ${slot.pinned ? "unpinned" : "pinned"}.`)}
              />
              <Button size="sm" variant="ghost" icon="trash" label="Remove" title={why ?? "Remove it from the board"} disabled={!!why || !!busy} onclick={() => remove(slot)} />
            </span>
          </div>
        </li>
      {/each}
    </ul>
    <div class="space" title="{bytes(used)} in slots · {bytes(room)} free for more · {bytes(set.reserve_bytes)} kept free">
      <div class="bar" aria-hidden="true">
        <span class="used" style="width: {total ? (used / total) * 100 : 0}%"></span>
        <span class="reserve" style="width: {total ? (Math.min(set.reserve_bytes, set.free_bytes) / total) * 100 : 0}%"></span>
      </div>
      <p class="text-[11px] text-fg-faint">{bytes(used)} in slots · {bytes(room)} free · {bytes(set.reserve_bytes)} kept free for data</p>
    </div>
  {/if}
</section>

{#if asking}
  <GuardedDialog
    {record}
    title={asking.title}
    body={asking.body}
    label={asking.label}
    busy={!!busy}
    onconfirm={() => asking && void run(asking.command, asking.command.command === "remove" ? "Removed." : `${deviceName(record)} is restarting into it.`)}
    oncancel={() => (asking = null)}
  />
{/if}

<style>
  .rows {
    display: flex;
    flex-direction: column;
  }
  .row {
    display: flex;
    flex-direction: column;
    gap: 2px;
    padding: 6px 0;
    border-top: 1px solid var(--hairline);
  }
  .row:first-child {
    border-top: 0;
  }
  .row.current {
    box-shadow: inset 2px 0 0 var(--success, var(--accent));
    padding-left: 8px;
  }
  .line {
    display: flex;
    align-items: center;
    gap: 6px;
    min-width: 0;
  }
  .flags,
  .acts {
    display: flex;
    flex: none;
    align-items: center;
    gap: 4px;
  }
  .flags {
    margin-left: auto;
  }
  .space {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .bar {
    display: flex;
    height: 6px;
    overflow: hidden;
    border-radius: 2px;
    background: var(--glass-hover);
  }
  .used {
    background: var(--accent);
  }
  .reserve {
    margin-left: auto;
    background: repeating-linear-gradient(135deg, var(--hairline) 0 3px, transparent 3px 6px);
  }
  .problem {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: 12.5px;
    color: var(--error-fg, var(--fg));
  }
</style>
