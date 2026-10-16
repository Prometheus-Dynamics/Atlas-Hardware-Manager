<script lang="ts">
  // The device's latest history (the board's event log and Atlas's record),
  // a few lines, newest first; the History tab has the rest.
  import { keyString, type DeviceRecord } from "#lib/api/client.ts";
  import { timeAgo } from "#lib/format.ts";
  import { clock } from "#lib/stores/clock.svelte.ts";
  import { deviceStatus } from "#lib/stores/status.svelte.ts";

  let { record, limit = 8, onmore }: { record: DeviceRecord; limit?: number; onmore?: () => void } = $props();

  const entries = $derived(
    [...(deviceStatus.history.get(keyString(record.key)) ?? [])].sort((a, b) => b.at_ms - a.at_ms).slice(0, limit),
  );
  const tone = (level: string) => (level === "error" ? "err" : level === "warning" ? "warn" : level === "success" ? "ok" : "");
</script>

<section class="panel" aria-label="Recent events">
  <div class="panel-head">
    <h2 class="panel-title">Recent events</h2>
    {#if onmore}<button type="button" class="more" onclick={onmore}>History</button>{/if}
  </div>
  {#if entries.length === 0}
    <p class="empty">Nothing recorded yet.</p>
  {:else}
    <ol>
      {#each entries as e, i (i)}
        <li>
          <span class="when">{timeAgo(e.at_ms, clock.now)}</span>
          <span class="kind {tone(e.level)}" title={e.kind}>{e.kind}</span>
          <span class="text" title={e.message}>{e.message}</span>
        </li>
      {/each}
    </ol>
  {/if}
</section>

<style>
  .panel {
    min-height: 0;
  }
  .more {
    font-size: 12px;
    color: var(--info-fg);
  }
  .more:hover {
    color: var(--fg);
  }
  ol {
    display: flex;
    flex-direction: column;
    min-height: 0;
    overflow: hidden;
    font-size: 12px;
  }
  li {
    display: grid;
    grid-template-columns: 58px 112px minmax(0, 1fr);
    gap: 8px;
    padding: 3px 0;
    border-top: 1px solid var(--hairline);
  }
  li:first-child {
    border-top: 0;
  }
  .when {
    color: var(--fg-faint);
    font-variant-numeric: tabular-nums;
  }
  .kind {
    font-family: var(--font-code);
    font-size: 11px;
    color: var(--fg-muted);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .kind.warn {
    color: var(--warn-fg);
  }
  .kind.err {
    color: var(--err-fg);
  }
  .kind.ok {
    color: var(--ok-fg);
  }
  .text {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .empty {
    font-size: 12.5px;
    color: var(--fg-muted);
  }
</style>
