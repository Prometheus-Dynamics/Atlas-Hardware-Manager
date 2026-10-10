<script lang="ts">
  // How everything is connected, drawn as a tree from this computer.
  // Live links carry a moving dash; offline ones fade.
  import { keyString, type DeviceRecord } from "#lib/api/client.ts";
  import Icon from "#lib/components/common/Icon.svelte";
  import StatusDot from "#lib/components/common/StatusDot.svelte";
  import { deviceName, primaryVersion } from "#lib/format.ts";
  import { metricValue } from "#lib/metrics.ts";
  import { deviceIcon, isRecovery } from "#lib/present.ts";
  import { insights } from "#lib/stores/insights.svelte.ts";
  import { jobs } from "#lib/stores/jobs.svelte.ts";
  import { live } from "#lib/stores/live.svelte.ts";
  import { ui } from "#lib/stores/ui.svelte.ts";
  import { HOST, layoutMap, type MapNode } from "./layout";

  let { records }: { records: DeviceRecord[] } = $props();

  const PAD = 8;
  // Room between columns for the wire and its link label ("USB boot").
  const GAP = 72;

  // The map fits the box it is given: rows spread out when there is height
  // to spare and tighten (down to a floor, then scroll) when there isn't.
  let width = $state(720);
  let boxHeight = $state(360);
  // Nodes widen on big screens so names and readings have room.
  const map = $derived(layoutMap(records));
  const NODE_W = $derived(Math.round(Math.min(220, Math.max(132, (width - PAD * 2 - map.depth * GAP) / (map.depth + 1)))));
  const ROW_H = $derived(Math.floor(Math.min(92, Math.max(44, (boxHeight - PAD * 2) / Math.max(1, map.rows)))));
  const NODE_H = $derived(Math.min(54, Math.max(36, ROW_H - 12)));
  // Narrow nodes drop the icon tile so the name keeps the room.
  const compact = $derived(NODE_W < 170);
  const colW = $derived(map.depth === 0 ? 0 : Math.max(NODE_W + GAP, (width - NODE_W - PAD * 2) / map.depth));
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

<div class="box" bind:clientWidth={width} bind:clientHeight={boxHeight}>
  <div class="relative my-auto shrink-0" style="height: {height}px; min-width: {map.depth * colW + NODE_W + PAD * 2}px">
    <svg class="absolute inset-0 h-full w-full" aria-hidden="true">
      {#each map.edges as edge (`${edge.from}>${edge.to}`)}
        {@const from = byId.get(edge.from)}
        {@const to = byId.get(edge.to)}
        {#if from && to}
          <path d={path(from, to)} class="wire" class:on={edge.online} />
        {/if}
      {/each}
      <!-- Labels over every wire, at the end of theirs, just before the
           device: wires that fan out of one node share their middle, not
           their ends. -->
      {#each map.edges as edge (`${edge.from}>${edge.to}`)}
        {@const to = byId.get(edge.to)}
        {#if to && edge.label && byId.has(edge.from)}
          <text x={x(to) - 8} y={y(to) - 5} class="tag">{edge.label}</text>
        {/if}
      {/each}
    </svg>

    {#each map.nodes as node (node.id)}
      {#if node.id === HOST}
        <div class="node host" class:compact style="left: {x(node)}px; top: {y(node) - NODE_H / 2}px; width: {NODE_W}px; height: {NODE_H}px">
          <span class="ico"><Icon name="device-laptop" size={17} stroke={1.7} /></span>
          <span class="min-w-0">
            <span class="name" title="This computer">This computer</span>
            <span class="sub">Atlas</span>
          </span>
        </div>
      {:else if node.record}
        {@const record = node.record}
        <button
          type="button"
          class="node"
          class:compact
          class:offline={record.presence !== "online"}
          class:attention={dotState(record) === "failed" || isRecovery(record)}
          style="left: {x(node)}px; top: {y(node) - NODE_H / 2}px; width: {NODE_W}px; height: {NODE_H}px"
          onclick={() => ui.openDevice(node.id)}
        >
          <span class="ico"><Icon name={deviceIcon(record)} size={17} stroke={1.7} /></span>
          <span class="min-w-0 flex-1">
            <span class="name" title={deviceName(record)}>{deviceName(record)}</span>
            <span class="sub" title={detail(record)}>{detail(record)}</span>
          </span>
          <StatusDot state={dotState(record)} />
        </button>
      {/if}
    {/each}
  </div>
</div>

<style>
  /* Fills the card body; the map sits centred in it (auto margins never
     push content out of reach the way centring would). */
  .box {
    position: absolute;
    inset: 6px 10px 10px;
    display: flex;
    flex-direction: column;
    overflow: auto;
  }
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
    text-anchor: end;
    letter-spacing: 0.02em;
    /* A halo in the card's colour keeps it readable where wires converge. */
    paint-order: stroke;
    stroke: var(--layer-solid);
    stroke-width: 4px;
    stroke-linejoin: round;
  }
  .node {
    position: absolute;
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 0 12px 0 8px;
    border-radius: var(--r-card);
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
  .node.compact {
    padding-left: 12px;
  }
  .node.compact .ico {
    display: none;
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
    border-radius: var(--r-md);
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
