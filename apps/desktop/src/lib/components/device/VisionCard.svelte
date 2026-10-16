<script lang="ts">
  // What the board's camera sees, live: the stream, the latency strip (on
  // the Styx builds PhotonVision's stages: exposure/2, readout, ISP,
  // detection, PhotonVision + NT; elsewhere just the total) and the tags
  // it found. From PhotonVision on the board, about ten results a second.
  import { api, type DeviceRecord, type VisionCamera } from "#lib/api/client.ts";
  import { deviceName } from "#lib/format.ts";
  import type { CameraStream } from "#lib/present.ts";
  import CameraStreams from "./CameraStreams.svelte";

  let { record, streams }: { record: DeviceRecord; streams: CameraStream[] } = $props();

  let cameras = $state<VisionCamera[]>([]);
  let waiting = $state<string | null>(null);
  let available = $state(true);

  $effect(() => {
    const key = record.key;
    if (record.presence !== "online") return;
    let stop: (() => void) | null = null;
    let gone = false;
    void api
      .streamVision(key, (frame) => {
        if (gone) return;
        if (frame.kind === "results") {
          cameras = frame.cameras;
          waiting = null;
        } else waiting = frame.reason;
      })
      .then((stopper) => {
        if (gone) stopper?.();
        else if (stopper) stop = stopper;
        else available = false;
      })
      .catch(() => (available = false));
    return () => {
      gone = true;
      stop?.();
    };
  });

  /** The camera the stream shows, by name; else the first. */
  const shown = $derived(cameras.find((c) => streams.some((s) => s.name.split(/[-_]/)[0] === c.camera.toLowerCase())) ?? cameras[0] ?? null);
  const SEGMENTS = [
    { key: "exposure_half", label: "exp/2", color: "var(--series-1)" },
    { key: "readout", label: "readout", color: "var(--series-8)" },
    { key: "isp", label: "ISP", color: "var(--series-4)" },
    { key: "detection", label: "detect", color: "var(--series-2)" },
    { key: "pv_nt", label: "PV+NT", color: "var(--series-3)" },
  ] as const;
  const segments = $derived.by(() => {
    const s = shown?.stages;
    const total = shown?.latency_ms ?? 0;
    if (!s || total <= 0) return [];
    const named = SEGMENTS.map((seg) => ({ ...seg, ms: s[seg.key] }));
    // The queue and anything unnamed: what's left of the total.
    const rest = Math.max(0, total - named.reduce((a, b) => a + b.ms, 0));
    const all = [...named, { key: "queue", label: "queue", color: "var(--series-5)", ms: rest }].filter((x) => x.ms > 0.005);
    return all.map((x) => ({ ...x, pct: (x.ms / total) * 100 }));
  });
  const ms = (v: number | null | undefined, d = 1) => (v === null || v === undefined ? "–" : v.toFixed(d));
</script>

<section class="panel vision" aria-label="Vision">
  {#if streams.length}
    <div class="camera"><CameraStreams {streams} device={deviceName(record)} single /></div>
  {/if}
  {#if available}
    <div class="lat">
      <div class="line">
        <span class="truncate">
          Latency <b class="mono">{ms(shown?.latency_ms)} ms</b> capture → NT{#if shown?.stages} · start of frame → NT <b class="mono">{ms(shown.stages.sof_to_nt)} ms</b>{/if}
        </span>
        <span class="mono shrink-0">{shown?.fps ? `${Math.round(shown.fps)} fps` : waiting ? "waiting" : "–"}</span>
      </div>
      {#if segments.length}
        <div class="strip" role="img" aria-label="Latency stages: {segments.map((s) => `${s.label} ${s.ms.toFixed(2)} ms`).join(', ')}">
          {#each segments as seg (seg.key)}
            <span style="width: {seg.pct}%; background: {seg.color}" title="{seg.label} {seg.ms.toFixed(2)} ms">{#if seg.pct > 12}{seg.label} {seg.ms.toFixed(seg.ms < 1 ? 2 : 1)}{/if}</span>
          {/each}
        </div>
      {:else if shown}
        <p class="note">Stages are on PhotonVision's Styx builds; this one reports the total only.</p>
      {:else if waiting}
        <p class="note truncate" title={waiting}>Waiting for PhotonVision: {waiting}</p>
      {/if}
    </div>
    <div class="tags">
      <div class="th"><span>Tag</span><span>Dist</span><span>Yaw</span><span>Pitch</span><span>Amb</span></div>
      {#each (shown?.targets ?? []).slice(0, 4) as tag, i (i)}
        <div class="tr mono"><span>{tag.id ?? "–"}</span><span>{tag.distance === null ? "–" : `${tag.distance.toFixed(2)} m`}</span><span>{ms(tag.yaw)}°</span><span>{ms(tag.pitch)}°</span><span>{ms(tag.ambiguity, 2)}</span></div>
      {:else}
        <p class="note">{shown ? "No tags in view." : "No results yet."}</p>
      {/each}
    </div>
  {/if}
</section>

<style>
  .vision {
    gap: 0;
    padding: 0;
    overflow: hidden;
  }
  .camera :global(.frame) {
    border: none;
    border-radius: 0;
  }
  .lat {
    display: flex;
    flex-direction: column;
    gap: 4px;
    padding: 6px 10px;
    border-top: 1px solid var(--hairline);
  }
  .line {
    display: flex;
    justify-content: space-between;
    gap: 8px;
    font-size: 11.5px;
    color: var(--fg-muted);
  }
  .line b {
    color: var(--fg);
    font-weight: 500;
  }
  .strip {
    display: flex;
    height: 14px;
    overflow: hidden;
    border-radius: var(--r-sm);
    font-size: 9.5px;
    font-weight: 600;
    line-height: 14px;
    color: #0f1115;
    white-space: nowrap;
  }
  .strip span {
    overflow: hidden;
    padding-left: 3px;
    text-overflow: clip;
  }
  .tags {
    padding: 4px 10px 6px;
    border-top: 1px solid var(--hairline);
  }
  .th,
  .tr {
    display: grid;
    grid-template-columns: 40px minmax(0, 1fr) minmax(0, 1fr) minmax(0, 1fr) minmax(0, 1fr);
    gap: 6px;
    height: 19px;
    align-items: center;
    font-size: 12px;
    white-space: nowrap;
  }
  .th {
    font-size: 10px;
    letter-spacing: 0.06em;
    text-transform: uppercase;
    color: var(--fg-faint);
  }
  .note {
    font-size: 11.5px;
    color: var(--fg-faint);
  }
</style>
