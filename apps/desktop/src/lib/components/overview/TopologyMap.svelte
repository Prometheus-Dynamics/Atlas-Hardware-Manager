<script lang="ts">
  // How everything is connected, drawn as a tree from this computer.
  // Live links carry a moving dash; offline ones fade.
  import { keyString, type DeviceRecord } from "$lib/api/client";
  import Icon from "$lib/components/common/Icon.svelte";
  import StatusDot from "$lib/components/common/StatusDot.svelte";
  import { deviceName, primaryVersion } from "$lib/format";
  import { metricValue } from "$lib/metrics";
  import { deviceIcon, isRecovery } from "$lib/present";
  import { insights } from "$lib/stores/insights.svelte";
  import { jobs } from "$lib/stores/jobs.svelte";
  import { live } from "$lib/stores/live.svelte";
  import { ui } from "$lib/stores/ui.svelte";
  import { HOST, layoutMap, type MapNode } from "./layout";

  let { records }: { records: DeviceRecord[] } = $props();

  const NODE_H = 48;
  const ROW_H = 64;
  const PAD = 8;

  let width = $state(720);
  // Nodes widen on big screens so names and readings have room.
  const NODE_W = $derived(Math.round(Math.min(260, Math.max(188, width / 5))));
  const map = $derived(layoutMap(records));
  const colW = $derived(map.depth === 0 ? 0 : Math.max(NODE_W + 56, (width - NODE_W - PAD * 2) / map.depth));
  const height = $derived(map.rows * ROW_H + PAD * 2);
  const byId = $derived(new Map(map.nodes.map((n) => [n.id, n])));

  const x = (n: MapNode) => PAD + n.depth * colW;
  const y = (n: MapNode) => PAD + n.row * ROW_H + ROW_H / 2;

  function path(from: MapNode, to: MapNode): string {
    const x0 = x(from) + NODE_W;
    const y0 = y(from);
    const x1 = x(to);
    const y1 = y(to);
    const mx = (x0 + x1) / 2;
    return `M${x0},${y0} C${mx},${y0} ${mx},${y1} ${x1},${y1}`;
  }

  function dotState(record: DeviceRecord): "online" | "busy" | "failed" | "offline" {
    const id = keyString(record.key);
    if (record.presence !== "online") return "offline";
    if (jobs.active.has(id)) return "busy";
    if (insights.failedIds.has(id)) return "failed";
    return "online";
  }

  /** A short live fact for the node: temperature, else version. */
  function detail(record: DeviceRecord): string {
    if (isRecovery(record)) return "Waiting for an image";
    if (record.presence !== "online") return "Offline";
    const temp = live.metric(keyString(record.key), "temp");
    const version = primaryVersion(record.identity);
    if (temp) {
      const t = metricValue(temp);
      return [version, `${t.value}${t.unit}`].filter(Boolean).join(" · ");
    }
    return version ?? record.identity.model;
  }
</script>

<div class="relative w-full overflow-x-auto" bind:clientWidth={width}>
  <div class="relative" style="height: {height}px; min-width: {map.depth * colW + NODE_W + PAD * 2}px">
    <svg class="absolute inset-0 h-full w-full" aria-hidden="true">
      {#each map.edges as edge (edge.from + edge.to)}
        {@const from = byId.get(edge.from)}
        {@const to = byId.get(edge.to)}
        {#if from && to}
          <path d={path(from, to)} class="wire" class:on={edge.online} />
          {#if edge.label}
            <text x={(x(from) + NODE_W + x(to)) / 2} y={(y(from) + y(to)) / 2 - 6} class="tag">{edge.label}</text>
          {/if}
        {/if}
      {/each}
    </svg>

    {#each map.nodes as node (node.id)}
      {#if node.id === HOST}
        <div class="node host" style="left: {x(node)}px; top: {y(node) - NODE_H / 2}px; width: {NODE_W}px; height: {NODE_H}px">
          <span class="ico"><Icon name="device-laptop" size={17} stroke={1.7} /></span>
          <span class="min-w-0">
            <span class="name">This computer</span>
            <span class="sub">Atlas</span>
          </span>
        </div>
      {:else if node.record}
        {@const record = node.record}
        <button
          type="button"
          class="node"
          class:offline={record.presence !== "online"}
          class:attention={dotState(record) === "failed" || isRecovery(record)}
          style="left: {x(node)}px; top: {y(node) - NODE_H / 2}px; width: {NODE_W}px; height: {NODE_H}px"
          onclick={() => ui.openDevice(node.id)}
        >
          <span class="ico"><Icon name={deviceIcon(record)} size={17} stroke={1.7} /></span>
          <span class="min-w-0 flex-1">
            <span class="name">{deviceName(record)}</span>
            <span class="sub">{detail(record)}</span>
          </span>
          <StatusDot state={dotState(record)} />
        </button>
      {/if}
    {/each}
  </div>
</div>

<style>
  .wire {
    fill: none;
    stroke: var(--glass-border);
    stroke-width: 1.5;
    stroke-dasharray: 3 5;
  }
  .wire.on {
    stroke: color-mix(in srgb, var(--ok) 55%, transparent);
    stroke-dasharray: none;
  }
  .tag {
    font-size: 10.5px;
    font-weight: 500;
    fill: var(--fg-faint);
    text-anchor: middle;
    letter-spacing: 0.02em;
  }
  .node {
    position: absolute;
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 0 12px 0 8px;
    border-radius: 12px;
    background: var(--layer-solid);
    border: 1px solid var(--glass-border);
    text-align: left;
    transition:
      transform var(--t-fast),
      border-color var(--t-fast),
      box-shadow var(--t-med),
      opacity var(--t-med);
  }
  button.node:hover {
    border-color: var(--glass-border-strong);
    box-shadow: var(--shadow-lift);
    transform: translateY(-1px);
  }
  button.node:active {
    transform: scale(0.98);
  }
  .node.offline {
    opacity: 0.55;
  }
  .node.attention {
    border-color: color-mix(in srgb, var(--accent) 55%, transparent);
    box-shadow: 0 0 0 3px var(--accent-tint);
  }
  .node.host {
    background: var(--glass-strong);
  }
  .ico {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 32px;
    height: 32px;
    flex-shrink: 0;
    border-radius: 9px;
    background: var(--glass);
    color: var(--fg);
  }
  .name {
    display: block;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: 13px;
    font-weight: 500;
    color: var(--fg);
  }
  .sub {
    display: block;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: 11.5px;
    color: var(--fg-faint);
    font-variant-numeric: tabular-nums;
  }
</style>
