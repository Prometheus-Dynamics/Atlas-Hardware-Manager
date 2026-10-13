<script lang="ts">
  // The device's own log, followed live while this tab is open: searched,
  // narrowed to one source or to problems, with the lines per minute (and
  // how many of them were problems) above it.
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
  let search = $state("");
  let source = $state<string | null>(null);
  let box: HTMLDivElement | undefined = $state();

  const bad = (l: LogLine) => l.level === "warning" || l.level === "error";
  const shown = $derived.by(() => {
    const q = search.trim().toLowerCase();
    return (lines ?? []).filter(
      (l) =>
        (filter === "all" || bad(l)) &&
        (source === null || l.source === source) &&
        (!q || l.message.toLowerCase().includes(q) || (l.source ?? "").toLowerCase().includes(q)),
    );
  });
  const problems = $derived((lines ?? []).filter(bad).length);
  // Each source with its line count, busiest first.
  const sources = $derived.by(() => {
    const n = new Map<string, number>();
    for (const l of lines ?? []) if (l.source) n.set(l.source, (n.get(l.source) ?? 0) + 1);
    return [...n].sort((a, b) => b[1] - a[1]);
  });
  // Lines per minute over what is loaded, problems apart.
  const minutes = $derived.by(() => {
    const timed = (lines ?? []).filter((l) => l.at_ms !== null);
    if (timed.length === 0) return [];
    const first = Math.floor(timed[0].at_ms! / 60_000);
    const last = Math.floor(timed[timed.length - 1].at_ms! / 60_000);
    const out = Array.from({ length: Math.min(60, last - first + 1) }, (_, i) => ({ at: (last - Math.min(59, last - first) + i) * 60_000, ok: 0, bad: 0 }));
    for (const l of timed) {
      const i = Math.floor(l.at_ms! / 60_000) - (last - out.length + 1);
      if (i < 0 || i >= out.length) continue;
      if (bad(l)) out[i].bad += 1;
      else out[i].ok += 1;
    }
    return out;
  });
  const peak = $derived(Math.max(1, ...minutes.map((m) => m.ok + m.bad)));

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
    <input class="input search" bind:value={search} placeholder="Search the log" aria-label="Search the log" spellcheck="false" />
    {#if sources.length > 1}
      <div class="sources" role="group" aria-label="Source">
        {#each sources as [name, n] (name)}
          <button type="button" class="chip" class:on={source === name} aria-pressed={source === name} onclick={() => (source = source === name ? null : name)}>
            {name}<span class="n">{n}</span>
          </button>
        {/each}
      </div>
    {/if}
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
    {#if minutes.length > 1}
      <div class="rate" role="img" aria-label="Lines per minute over the last {minutes.length} minutes">
        {#each minutes as m (m.at)}
          <span class="bar" title="{time(m.at)}: {m.ok + m.bad} lines, {m.bad} warnings or errors">
            <span class="b" style:height="{(m.bad / peak) * 100}%"></span>
            <span class="o" style:height="{(m.ok / peak) * 100}%"></span>
          </span>
        {/each}
      </div>
    {/if}
    <div class="log" bind:this={box}>
      {#each shown as line, i (i)}
        <div class="row {line.level}">
          <span class="t">{time(line.at_ms)}</span>
          <span class="lv">{LEVEL[line.level]}</span>
          {#if line.source}<span class="src">{line.source}</span>{/if}
          <span class="msg">{line.message}</span>
        </div>
      {:else}
        <p class="px-4 py-6 text-center text-fg-faint">
          {search || source ? "No line matches." : filter === "all" ? "Nothing logged yet." : "No warnings or errors."}
        </p>
      {/each}
    </div>
    {#if !following}<p class="hint">Paused. New lines appear when you follow again.</p>{/if}
  {/if}
</div>

<style>
  .search {
    width: 14rem;
    height: 30px;
    font-size: 12.5px;
  }
  .sources {
    display: flex;
    flex-wrap: wrap;
    gap: 4px;
  }
  .chip {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    height: 26px;
    padding: 0 8px;
    font-size: 12px;
    color: var(--fg-muted);
    border: 1px solid var(--glass-border);
    border-radius: var(--r-pill);
  }
  .chip:hover {
    color: var(--fg);
  }
  .chip.on {
    color: var(--fg);
    border-color: var(--accent-ring);
    background: var(--accent-tint);
  }
  .n {
    font-variant-numeric: tabular-nums;
    color: var(--fg-faint);
  }
  .rate {
    display: flex;
    align-items: stretch;
    gap: 1px;
    height: 24px;
  }
  .bar {
    flex: 1;
    display: flex;
    flex-direction: column-reverse;
    background: var(--inset);
    border-radius: 1px;
  }
  .bar .o {
    background: color-mix(in srgb, var(--series-1) 70%, transparent);
  }
  .bar .b {
    background: var(--warn);
  }
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
  .row.warning {
    background: color-mix(in srgb, var(--warn) 8%, transparent);
  }
  .row.error {
    background: color-mix(in srgb, var(--err) 10%, transparent);
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
