<script lang="ts">
  // A timeline of fleet history: what came and went, what was updated.
  // Entries about a device open it.
  import { keyString, type ActivityEntry, type ActivityKind, type ActivityLevel } from "#lib/api/client.ts";
  import Icon from "#lib/components/common/Icon.svelte";
  import { timeAgo } from "#lib/format.ts";
  import { clock } from "#lib/stores/clock.svelte.ts";
  import { devices } from "#lib/stores/devices.svelte.ts";
  import { ui } from "#lib/stores/ui.svelte.ts";
  import type { IconName } from "#lib/ui/icons.ts";
  import { rise, stagger } from "#lib/ui/motion.ts";

  let { entries, limit = 12, empty = "Nothing has happened yet." }: { entries: ActivityEntry[]; limit?: number; empty?: string } = $props();

  let expanded = $state(false);

  /** A row: one entry, or a burst of the same thing (a scan finding eight devices). */
  type Row = { entry: ActivityEntry; count: number; names: string[]; key: string };

  const GROUPABLE = new Set<ActivityKind>(["device-found", "device-online", "device-offline"]);
  const BURST_MS = 30_000;

  function subject(entry: ActivityEntry): string {
    return entry.device ? devices.nameOf(entry.device) : entry.message;
  }

  const rows = $derived.by(() => {
    const out: Row[] = [];
    // Two entries can match in time and text (a board that staged the same
    // version twice, caught up in one batch): the nth such gets #n.
    const seen = new Map<string, number>();
    for (const entry of entries) {
      const last = out.at(-1);
      if (
        last &&
        GROUPABLE.has(entry.kind) &&
        last.entry.kind === entry.kind &&
        Math.abs(last.entry.at_ms - entry.at_ms) < BURST_MS
      ) {
        last.count++;
        last.names.push(subject(entry));
      } else {
        const base = `${entry.at_ms}|${entry.kind}|${entry.message}`;
        const n = seen.get(base) ?? 0;
        seen.set(base, n + 1);
        out.push({ entry, count: 1, names: [subject(entry)], key: n ? `${base}#${n}` : base });
      }
    }
    return out;
  });
  const shown = $derived(expanded ? rows : rows.slice(0, limit));

  const VERB: Partial<Record<ActivityKind, string>> = {
    "device-found": "Found",
    "device-online": "Back online:",
    "device-offline": "Went offline:",
  };

  function text(row: Row): string {
    if (row.count === 1) return row.entry.message;
    const names = row.names.length > 3 ? `${row.names.slice(0, 3).join(", ")} and ${row.names.length - 3} more` : row.names.join(", ");
    return `${VERB[row.entry.kind] ?? ""} ${row.count} devices: ${names}`.trim();
  }

  const ICON: Record<ActivityKind, IconName> = {
    "device-found": "sparkles",
    "device-online": "plug-connected",
    "device-offline": "plug-connected-x",
    "version-changed": "arrow-up",
    "mode-changed": "usb",
    "update-result": "package",
    "action-run": "player-play",
    "self-test": "list-check",
    "device-event": "terminal-2",
  };

  function icon(entry: ActivityEntry): IconName {
    if (entry.kind === "update-result" && entry.level === "success") return "circle-check";
    if (entry.kind === "update-result") return "alert-triangle";
    return ICON[entry.kind] ?? "info-circle";
  }

  const TONE: Record<ActivityLevel, string> = { info: "info", success: "ok", warning: "warn", error: "err" };

  function open(entry: ActivityEntry) {
    if (entry.device && devices.get(entry.device)) ui.openDevice(keyString(entry.device));
  }
</script>

{#if entries.length === 0}
  <p class="py-6 text-center text-[13px] text-fg-faint">{empty}</p>
{:else}
  <ol class="feed">
    {#each shown as row, i (row.key)}
      {@const entry = row.entry}
      {@const clickable = row.count === 1 && !!entry.device && !!devices.get(entry.device)}
      <li in:rise={{ delay: stagger(i) }}>
        <button type="button" class="item" disabled={!clickable} onclick={() => open(entry)}>
          <span class="mark {TONE[entry.level]}"><Icon name={icon(entry)} size={13} stroke={2} /></span>
          <span class="min-w-0 flex-1 text-[13px] leading-snug text-fg">{text(row)}</span>
          <span class="shrink-0 text-[11.5px] text-fg-faint" title={new Date(entry.at_ms).toLocaleString()}>{timeAgo(entry.at_ms, clock.now)}</span>
        </button>
      </li>
    {/each}
  </ol>
  {#if rows.length > limit}
    <button type="button" class="link mt-1 self-start text-[12.5px]" onclick={() => (expanded = !expanded)}>
      {expanded ? "Show less" : `Show all ${rows.length}`}
    </button>
  {/if}
{/if}

<style>
  .feed {
    position: relative;
    display: flex;
    flex-direction: column;
  }
  /* The thread joining the markers. */
  .feed::before {
    content: "";
    position: absolute;
    left: 19px;
    top: 14px;
    bottom: 14px;
    width: 1px;
    background: var(--hairline);
  }
  .item {
    position: relative;
    display: flex;
    align-items: center;
    gap: 12px;
    width: 100%;
    padding: 7px 8px;
    border-radius: var(--r-md);
    text-align: left;
    transition: background var(--t-fast);
  }
  .item:not(:disabled):hover {
    background: var(--glass);
  }
  .item:disabled {
    cursor: default;
  }
  .mark {
    position: relative;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 24px;
    height: 24px;
    flex-shrink: 0;
    border-radius: 50%;
    border: 1px solid var(--glass-border);
    background: var(--layer-solid);
  }
  .mark.info {
    color: var(--info-fg);
  }
  .mark.ok {
    color: var(--ok-fg);
    background: color-mix(in srgb, var(--ok) 14%, var(--layer-solid));
  }
  .mark.warn {
    color: var(--warn-fg);
    background: color-mix(in srgb, var(--warn) 14%, var(--layer-solid));
  }
  .mark.err {
    color: var(--err-fg);
    background: color-mix(in srgb, var(--err) 14%, var(--layer-solid));
  }
</style>
