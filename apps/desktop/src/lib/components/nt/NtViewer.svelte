<script lang="ts">
  // A live NetworkTables viewer: connect to a team's robot (its number) or
  // any NT4 server (a camera's own, over USB or the network), browse the
  // topic tree with live values, search it, and graph number topics.
  import { api, errorText, type NtFrame, type NtValue } from "#lib/api/client.ts";
  import Button from "#lib/components/common/Button.svelte";
  import GlassCard from "#lib/components/common/GlassCard.svelte";
  import Icon from "#lib/components/common/Icon.svelte";
  import Pill from "#lib/components/common/Pill.svelte";
  import SegmentedControl from "#lib/components/common/SegmentedControl.svelte";
  import LiveChart from "#lib/components/device/LiveChart.svelte";
  import { LiveSeries, WINDOWS } from "#lib/components/device/live.ts";
  import { clock } from "#lib/stores/clock.svelte.ts";
  import { onDestroy } from "svelte";
  import { SvelteMap, SvelteSet } from "svelte/reactivity";
  import { formatNt, isNumberType, numberOf, treeRows } from "./nt.ts";

  const RECENT_KEY = "atlas.nt.recent";
  const loadRecent = (): string[] => {
    try {
      return JSON.parse(localStorage.getItem(RECENT_KEY) ?? "[]") as string[];
    } catch {
      return [];
    }
  };

  let target = $state("");
  let port = $state(5810);
  let recent = $state<string[]>(loadRecent());
  let stop: (() => void) | null = null;
  /** A viewer is open (connected or trying). */
  let open = $state(false);
  let status = $state<{ connected: boolean; host: string; port: number; reason: string | null } | null>(null);
  let search = $state("");
  const collapsed = new SvelteSet<string>();

  /** Each topic's type, newest value and when it arrived (this computer's ms); not reactive per value. */
  const topics = new Map<string, { type: string; value: NtValue | null; at: number }>();
  /** Bumped once per frame, so the tree re-reads the values. */
  let version = $state(0);
  const graphs = new SvelteMap<string, LiveSeries>();
  let windowKey = $state("10");
  const windowS = $derived(Number(windowKey));

  function onFrame(frame: NtFrame) {
    if (frame.type === "status") {
      status = frame;
    } else if (frame.type === "topics") {
      for (const topic of frame.announced) {
        const known = topics.get(topic.name);
        topics.set(topic.name, { type: topic.type, value: known?.value ?? null, at: known?.at ?? 0 });
      }
      for (const name of frame.gone) {
        topics.delete(name);
        graphs.delete(name);
      }
    } else {
      const now = Date.now();
      for (const { name, t_us, value } of frame.values) {
        const topic = topics.get(name) ?? { type: value.type, value: null, at: 0 };
        topic.value = value;
        topic.at = now;
        topics.set(name, topic);
        const series = graphs.get(name);
        if (series) series.push(t_us, [numberOf(value)]);
      }
    }
    version += 1;
  }

  async function connect() {
    const to = target.trim();
    if (!to) return;
    disconnect();
    topics.clear();
    graphs.clear();
    stop = await api.ntConnect(to, port === 5810 ? null : port, onFrame);
    open = true;
    recent = [to, ...recent.filter((r) => r !== to)].slice(0, 6);
    try {
      localStorage.setItem(RECENT_KEY, JSON.stringify(recent));
    } catch {
      // Private mode: the list lasts this session.
    }
  }

  function disconnect() {
    stop?.();
    stop = null;
    open = false;
    status = null;
  }
  onDestroy(disconnect);

  function toggleGraph(name: string) {
    if (graphs.has(name)) {
      graphs.delete(name);
      return;
    }
    if (graphs.size >= 8) return;
    graphs.set(name, new LiveSeries({ id: name, channels: [{ name, unit: "" }], period_ms: 0 }));
  }

  function toggleFolder(path: string) {
    if (collapsed.has(path)) collapsed.delete(path);
    else collapsed.add(path);
  }

  const rows = $derived.by(() => {
    void version;
    return treeRows([...topics.keys()], collapsed, search);
  });
  const count = $derived.by(() => {
    void version;
    return topics.size;
  });
  const age = (at: number) => {
    if (!at) return "";
    const s = Math.max(0, (clock.now - at) / 1000);
    return s < 1 ? "now" : s < 60 ? `${Math.round(s)} s` : `${Math.round(s / 60)} min`;
  };
  /** A topic as its row shows it, made fresh each time (the store isn't reactive). */
  function shown(name: string, _version: number, _now: number) {
    const topic = topics.get(name);
    return { type: topic?.type ?? "", text: formatNt(topic?.value), age: age(topic?.at ?? 0) };
  }
  const windowOptions = WINDOWS.map((s) => ({ value: String(s), label: `${s} s` }));
  const statusPill = $derived(
    !status
      ? null
      : status.connected
        ? { tone: "success" as const, label: `Connected to ${status.host}:${status.port}` }
        : { tone: "warning" as const, label: `${status.reason === "connecting" ? "Connecting to" : "Reconnecting to"} ${status.host}:${status.port}` },
  );
</script>

<div class="flex min-h-0 flex-1 flex-col gap-4">
  <GlassCard>
    <form
      class="flex flex-wrap items-end gap-3"
      onsubmit={(event) => {
        event.preventDefault();
        void connect().catch((error) => (status = { connected: false, host: target, port, reason: errorText(error) }));
      }}
    >
      <label class="flex min-w-[14rem] flex-1 flex-col gap-1">
        <span class="text-[12px] text-fg-faint">Team number or address</span>
        <input class="input" bind:value={target} placeholder="5338, 10.53.38.2, photonvision.local" spellcheck="false" aria-label="Team number or address" />
      </label>
      <label class="flex w-24 flex-col gap-1">
        <span class="text-[12px] text-fg-faint">Port</span>
        <input class="input mono" type="number" min="1" max="65535" bind:value={port} aria-label="Port" />
      </label>
      {#if open}
        <Button icon="x" onclick={disconnect}>Disconnect</Button>
      {/if}
      <Button variant="primary" icon="plug-connected" type="submit" disabled={!target.trim()}>Connect</Button>
    </form>
    <div class="mt-3 flex flex-wrap items-center gap-2">
      {#if statusPill}<Pill tone={statusPill.tone} label={statusPill.label} />{/if}
      {#if status && !status.connected && status.reason && status.reason !== "connecting"}
        <span class="text-[12px] text-fg-faint">{status.reason}</span>
      {/if}
      {#each recent as item (item)}
        <button type="button" class="chip" onclick={() => ((target = item), void connect())}>{item}</button>
      {/each}
      <span class="ml-auto text-[12px] text-fg-faint">A team number means its robot (10.TE.AM.2).</span>
    </div>
  </GlassCard>

  {#if open}
    <div class="grid min-h-0 flex-1 gap-4 lg:grid-cols-[minmax(0,1.1fr)_minmax(0,1fr)]">
      <GlassCard title="Topics" subtitle="{count} live" fill>
        {#snippet actions()}
          <input class="input search" bind:value={search} placeholder="Search" aria-label="Search topics" />
        {/snippet}
        <div class="tree" role="tree" aria-label="NetworkTables topics">
          {#if rows.length === 0}
            <p class="p-3 text-[13px] text-fg-muted">{status?.connected ? (search ? "No topic matches." : "No topics yet.") : "Waiting for the server…"}</p>
          {/if}
          {#each rows as row (row.path)}
            {#if row.folder}
              <button type="button" class="row folder" style="--depth: {row.depth}" onclick={() => toggleFolder(row.path)} role="treeitem" aria-selected="false" aria-expanded={!collapsed.has(row.path)}>
                <Icon name={collapsed.has(row.path) ? "chevron-right" : "chevron-down"} size={14} />
                <span class="label">{row.label}</span>
              </button>
            {:else}
              {@const topic = shown(row.path, version, clock.now)}
              {@const graphable = isNumberType(topic.type) || topic.type === "boolean"}
              <div class="row topic" style="--depth: {row.depth}" role="treeitem" aria-selected={graphs.has(row.path)} title={row.path}>
                <span class="label">{row.label}</span>
                <span class="type">{topic.type}</span>
                <span class="value mono">{topic.text}</span>
                <span class="age">{topic.age}</span>
                {#if graphable}
                  <button type="button" class="graph" class:on={graphs.has(row.path)} onclick={() => toggleGraph(row.path)} aria-label="{graphs.has(row.path) ? 'Stop graphing' : 'Graph'} {row.path}" title={graphs.has(row.path) ? "Stop graphing" : "Graph"}>
                    <Icon name="chart-line" size={14} />
                  </button>
                {:else}
                  <span class="graph-space"></span>
                {/if}
              </div>
            {/if}
          {/each}
        </div>
      </GlassCard>

      <GlassCard title="Graphs" subtitle={graphs.size ? `${graphs.size} of 8` : "Pick number topics with their graph button"} fill>
        {#snippet actions()}
          {#if graphs.size}<SegmentedControl options={windowOptions} bind:value={windowKey} label="Graph window" size="sm" />{/if}
        {/snippet}
        <div class="flex flex-col gap-4 overflow-y-auto">
          {#each [...graphs] as [name, series] (name)}
            <div>
              <p class="mb-1 flex items-center gap-2 text-[12.5px] text-fg-muted">
                <span class="mono min-w-0 flex-1 truncate">{name}</span>
                <button type="button" class="graph on" onclick={() => graphs.delete(name)} aria-label="Stop graphing {name}"><Icon name="x" size={13} /></button>
              </p>
              <LiveChart {series} channels={[0]} unit="" {windowS} />
            </div>
          {/each}
        </div>
      </GlassCard>
    </div>
  {/if}
</div>

<style>
  .search {
    width: 12rem;
    height: 30px;
    font-size: 12.5px;
  }
  .tree {
    overflow-y: auto;
    min-height: 0;
    flex: 1;
    font-size: 13px;
  }
  .row {
    display: flex;
    align-items: center;
    gap: 8px;
    width: 100%;
    padding: 4px 8px 4px calc(8px + var(--depth) * 16px);
    border-radius: var(--r-md);
    text-align: left;
  }
  .row:hover {
    background: var(--glass-strong);
  }
  .folder {
    color: var(--fg-muted);
    font-weight: 500;
  }
  .topic .label {
    min-width: 0;
    flex: 0 1 auto;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: var(--fg);
  }
  .type {
    font-size: 11px;
    color: var(--fg-faint);
    white-space: nowrap;
  }
  .value {
    margin-left: auto;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-variant-numeric: tabular-nums;
    color: var(--fg);
    text-align: right;
  }
  .age {
    width: 3.5rem;
    text-align: right;
    font-size: 11px;
    color: var(--fg-faint);
    white-space: nowrap;
  }
  .graph,
  .graph-space {
    width: 24px;
    height: 24px;
    flex-shrink: 0;
  }
  .graph {
    display: grid;
    place-items: center;
    border-radius: var(--r-md);
    color: var(--fg-faint);
  }
  .graph:hover,
  .graph.on {
    color: var(--fg);
    background: var(--glass-strong);
  }
  .chip {
    padding: 2px 9px;
    border-radius: var(--r-round);
    font-size: 12px;
    color: var(--fg-muted);
    background: var(--glass-strong);
  }
  .chip:hover {
    color: var(--fg);
  }
</style>
