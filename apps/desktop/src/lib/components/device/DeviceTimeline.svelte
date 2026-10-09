<script lang="ts">
  // One device's merged history: what Atlas did and what the board's own
  // event log says, each with who did it (you via Atlas, Orion, on the
  // board). Same look as the fleet feed.
  import type { ActivityLevel, EventSource, HistoryEntry } from "#lib/api/client.ts";
  import Icon from "#lib/components/common/Icon.svelte";
  import { timeAgo } from "#lib/format.ts";
  import { clock } from "#lib/stores/clock.svelte.ts";
  import { sourceLabel } from "#lib/stores/status.svelte.ts";
  import type { IconName } from "#lib/ui/icons.ts";
  import { rise, stagger } from "#lib/ui/motion.ts";

  let { entries, limit = 15 }: { entries: HistoryEntry[]; limit?: number } = $props();

  let expanded = $state(false);
  const shown = $derived(expanded ? entries : entries.slice(0, limit));

  function icon(entry: HistoryEntry): IconName {
    const kind = entry.kind;
    if (kind === "boot" || kind === "reboot" || kind === "shutdown") return "refresh";
    if (kind === "power-off") return "power";
    if (kind === "clock.set") return "clock";
    if (kind === "ssh.keys") return "key";
    if (kind === "usb-boot" || kind === "mode-changed") return "usb";
    if (kind === "selftest" || kind === "self-test") return "list-check";
    if (kind === "update.rollback" || kind === "update.rolled-back" || kind === "update.link-fallback") return "history";
    if (kind === "update.cancelled") return "player-stop";
    if (kind === "update.download") return "download";
    if (kind.startsWith("update.") || kind === "update-result" || kind === "version-changed") {
      return entry.level === "success" ? "circle-check" : entry.level === "error" ? "alert-triangle" : "package";
    }
    if (kind === "device-online" || kind === "device-found") return "plug-connected";
    if (kind === "device-offline") return "plug-connected-x";
    if (kind === "action-run") return "player-play";
    return "info-circle";
  }

  const TONE: Record<ActivityLevel, string> = { info: "info", success: "ok", warning: "warn", error: "err" };
  const SOURCE_ICON: Record<EventSource, IconName> = {
    atlas: "device-laptop",
    orion: "router",
    local: "terminal-2",
    unknown: "info-circle",
  };

  function key(entry: HistoryEntry, i: number): string {
    return `${entry.origin}:${entry.boot_id ?? ""}:${entry.at_ms}:${entry.kind}:${i}`;
  }
</script>

{#if entries.length === 0}
  <p class="py-6 text-center text-[13px] text-fg-faint">Nothing has happened to this device yet.</p>
{:else}
  <ol class="feed">
    {#each shown as entry, i (key(entry, i))}
      <li in:rise={{ delay: stagger(Math.min(i, 12)) }} class="item">
        <span class="mark {TONE[entry.level]}"><Icon name={icon(entry)} size={13} stroke={2} /></span>
        <span class="min-w-0 flex-1 text-[13px] leading-snug text-fg">
          {entry.message}
          <span class="source" title={entry.origin === "board" ? "From the board's event log" : "Recorded on this computer"}>
            <Icon name={SOURCE_ICON[entry.source] ?? "info-circle"} size={11} />{sourceLabel(entry.source)}
          </span>
        </span>
        <span class="shrink-0 text-[11.5px] text-fg-faint" title={new Date(entry.at_ms).toLocaleString()}>{timeAgo(entry.at_ms, clock.now)}</span>
      </li>
    {/each}
  </ol>
  {#if entries.length > limit}
    <button type="button" class="link mt-1 self-start text-[12.5px]" onclick={() => (expanded = !expanded)}>
      {expanded ? "Show less" : `Show all ${entries.length}`}
    </button>
  {/if}
{/if}

<style>
  .feed {
    position: relative;
    display: flex;
    flex-direction: column;
  }
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
    padding: 7px 8px;
  }
  .source {
    display: inline-flex;
    align-items: center;
    gap: 3px;
    margin-left: 6px;
    padding: 0 6px;
    border-radius: var(--r-pill);
    background: var(--neutral-bg);
    color: var(--fg-muted);
    font-size: 11px;
    vertical-align: 1px;
    white-space: nowrap;
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
