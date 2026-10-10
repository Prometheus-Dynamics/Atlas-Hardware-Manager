<script lang="ts">
  // The device's own log, followed live while this tab is open.
  import { api, errorText, type DeviceRecord, type LogLevel, type LogLine } from "#lib/api/client.ts";
  import Button from "#lib/components/common/Button.svelte";
  import Icon from "#lib/components/common/Icon.svelte";
  import SegmentedControl from "#lib/components/common/SegmentedControl.svelte";
  import Skeleton from "#lib/components/common/Skeleton.svelte";
  import { toasts } from "#lib/stores/toasts.svelte.ts";
  import { saveSupportBundle } from "./supportBundle";

  let { record }: { record: DeviceRecord } = $props();

  const POLL_MS = 2000;
  const LINES = 300;

  let lines = $state<LogLine[] | null>(null);
  let error = $state<string | null>(null);
  let following = $state(true);
  let filter = $state("all");
  let box: HTMLDivElement | undefined = $state();

  const shown = $derived(
    (lines ?? []).filter((l) => filter === "all" || l.level === "warning" || l.level === "error"),
  );
  const problems = $derived((lines ?? []).filter((l) => l.level === "warning" || l.level === "error").length);

  $effect(() => {
    const key = record.key;
    if (!following) return;
    let stale = false;
    const load = async () => {
      try {
        const next = await api.deviceLogs(key, LINES);
        if (!stale) {
          lines = next;
          error = null;
        }
      } catch (e) {
        if (!stale) error = errorText(e);
      }
    };
    void load();
    const timer = setInterval(load, POLL_MS);
    return () => {
      stale = true;
      clearInterval(timer);
    };
  });

  // Stay at the bottom while following.
  $effect(() => {
    void shown.length;
    if (following && box) box.scrollTop = box.scrollHeight;
  });

  function time(ms: number | null): string {
    if (ms === null) return "";
    return new Date(ms).toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit", second: "2-digit", hour12: false });
  }

  const LEVEL: Record<LogLevel, string> = { debug: "dbg", info: "inf", warning: "wrn", error: "err" };

  async function copy() {
    const text = shown.map((l) => `${time(l.at_ms)} ${LEVEL[l.level]} ${l.source ? `[${l.source}] ` : ""}${l.message}`).join("\n");
    try {
      await navigator.clipboard.writeText(text);
      toasts.success("Log copied.");
    } catch {
      toasts.error("Couldn't copy to the clipboard.");
    }
  }
</script>

<!-- Fills the tab; only the log scrolls. -->
<div class="flex min-h-0 flex-1 basis-0 flex-col gap-3">
  <div class="flex flex-wrap items-center gap-2">
    <SegmentedControl
      options={[
        { value: "all", label: "All" },
        { value: "problems", label: problems ? `Problems (${problems})` : "Problems" },
      ]}
      bind:value={filter}
      label="Log filter"
    />
    <div class="ml-auto flex gap-1">
      <Button variant="ghost" size="sm" icon={following ? "player-pause" : "player-play"} onclick={() => (following = !following)}>
        {following ? "Pause" : "Follow"}
      </Button>
      <Button variant="ghost" size="sm" icon="copy" disabled={shown.length === 0} onclick={copy}>Copy</Button>
      <Button variant="ghost" size="sm" icon="device-floppy" action={() => saveSupportBundle(record)}>Save bundle</Button>
    </div>
  </div>

  {#if error && !lines}
    <p class="flex items-center gap-2 text-[13px] text-warn-fg"><Icon name="alert-triangle" size={15} />{error}</p>
  {:else if !lines}
    <div class="flex flex-col gap-2">{#each [0, 1, 2, 3, 4] as i (i)}<Skeleton height={14} />{/each}</div>
  {:else}
    <div class="log" bind:this={box}>
      {#each shown as line, i (i)}
        <div class="row {line.level}">
          <span class="t">{time(line.at_ms)}</span>
          <span class="lv">{LEVEL[line.level]}</span>
          {#if line.source}<span class="src">{line.source}</span>{/if}
          <span class="msg">{line.message}</span>
        </div>
      {:else}
        <p class="px-4 py-6 text-center text-fg-faint">{filter === "all" ? "Nothing logged yet." : "No warnings or errors."}</p>
      {/each}
    </div>
    {#if !following}<p class="hint">Paused. New lines appear when you follow again.</p>{/if}
  {/if}
</div>

<style>
  .log {
    flex: 1 1 0;
    min-height: 220px;
    overflow: auto;
    padding: 8px 0;
    border-radius: var(--r-card);
    background: var(--inset);
    border: 1px solid var(--hairline);
    font-family: var(--font-code);
    font-size: 11.5px;
    line-height: 1.6;
  }
  .row {
    display: flex;
    gap: 8px;
    padding: 0 14px;
    color: var(--fg-muted);
    white-space: pre-wrap;
  }
  .row:hover {
    background: var(--glass);
  }
  .t {
    color: var(--fg-faint);
    flex-shrink: 0;
  }
  .lv {
    flex-shrink: 0;
    width: 2.4em;
    color: var(--fg-faint);
  }
  .src {
    flex-shrink: 0;
    color: var(--info-fg);
  }
  .msg {
    min-width: 0;
    word-break: break-word;
  }
  .row.debug .msg {
    color: var(--fg-faint);
  }
  .row.warning .lv,
  .row.warning .msg {
    color: var(--warn-fg);
  }
  .row.error .lv,
  .row.error .msg {
    color: var(--err-fg);
  }
</style>
