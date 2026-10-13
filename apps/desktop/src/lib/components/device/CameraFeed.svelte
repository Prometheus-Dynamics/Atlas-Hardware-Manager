<script lang="ts">
  // One camera view that heals itself. Atlas follows the stream (it notices
  // a stream that broke or stalled, closes it and reconnects with backoff,
  // one connection at a time) and the view draws each frame. When the
  // stream is lost the last good frame stays, dimmed, under "Reconnecting";
  // it comes back by itself when the camera does (PhotonVision restarting,
  // a reboot, the cable). A URL Atlas doesn't stream (a still image, the
  // browser preview) is shown directly and retried after an error.
  import { api, type CameraState } from "#lib/api/client.ts";

  let {
    src,
    alt,
    fit = "cover",
    phase = $bindable<"connecting" | "live" | "reconnecting">("connecting"),
  }: { src: string; alt: string; fit?: "cover" | "contain"; phase?: "connecting" | "live" | "reconnecting" } = $props();

  let canvas = $state<HTMLCanvasElement>();
  let direct = $state(false);
  let retry = $state(0);
  let reason = $state("");
  let hasFrame = $state(false);
  let last: HTMLImageElement | null = null;
  let pending: Blob | null = null;
  let decoding = false;

  function draw() {
    if (!canvas || !last) return;
    const ratio = window.devicePixelRatio || 1;
    const width = Math.max(1, Math.round(canvas.clientWidth * ratio));
    const height = Math.max(1, Math.round(canvas.clientHeight * ratio));
    if (canvas.width !== width || canvas.height !== height) {
      canvas.width = width;
      canvas.height = height;
    }
    const context = canvas.getContext("2d");
    if (!context) return;
    const scale = (fit === "cover" ? Math.max : Math.min)(width / last.naturalWidth, height / last.naturalHeight);
    const w = last.naturalWidth * scale;
    const h = last.naturalHeight * scale;
    context.clearRect(0, 0, width, height);
    context.drawImage(last, (width - w) / 2, (height - h) / 2, w, h);
  }

  /** Decodes the newest frame; frames that arrive meanwhile replace each other. */
  async function frame(jpeg: Blob) {
    if (decoding) {
      pending = jpeg;
      return;
    }
    decoding = true;
    let next: Blob | null = jpeg;
    while (next) {
      pending = null;
      const url = URL.createObjectURL(next);
      const image = new Image();
      image.src = url;
      try {
        await image.decode();
        last = image;
        hasFrame = true;
        draw();
      } catch {
        // A frame that doesn't decode is skipped.
      } finally {
        URL.revokeObjectURL(url);
      }
      next = pending;
    }
    decoding = false;
  }

  function onCameraState(next: CameraState) {
    if (next.state === "live") {
      phase = "live";
      reason = "";
    } else if (next.state === "reconnecting") {
      phase = "reconnecting";
      reason = next.reason;
    } else if (phase !== "reconnecting") {
      phase = "connecting";
    }
  }

  $effect(() => {
    const url = src;
    let stop: (() => void) | null = null;
    let gone = false;
    phase = "connecting";
    hasFrame = false;
    direct = false;
    last = null;
    void api
      .streamCamera(url, (jpeg) => void frame(jpeg), onCameraState)
      .then((stopper) => {
        if (gone) stopper?.();
        else if (stopper) stop = stopper;
        else direct = true;
      })
      .catch(() => (direct = true));
    return () => {
      gone = true;
      stop?.();
    };
  });

  $effect(() => {
    if (!canvas) return;
    const observer = new ResizeObserver(() => draw());
    observer.observe(canvas);
    return () => observer.disconnect();
  });

  // A direct view that failed: try again in a moment, keeping the note up.
  function failedDirect() {
    phase = "reconnecting";
    reason = "the camera isn't answering";
    setTimeout(() => (retry += 1), 3000);
  }
</script>

<div class="feed" class:contain={fit === "contain"} role="img" aria-label={alt}>
  {#if direct}
    {#key retry}
      <img
        {src}
        {alt}
        class:dim={phase === "reconnecting"}
        onload={() => ((phase = "live"), (hasFrame = true))}
        onerror={failedDirect}
      />
    {/key}
  {:else}
    <canvas bind:this={canvas} class:dim={phase === "reconnecting"}></canvas>
  {/if}
  {#if !hasFrame}
    <div class="note">
      {#if phase === "reconnecting"}
        <span class="spin"></span>
        <span>The camera isn't answering; trying again</span>
        {#if reason}<span class="why">{reason}</span>{/if}
      {:else}
        <div class="shimmer absolute inset-0"></div>
      {/if}
    </div>
  {:else if phase === "reconnecting"}
    <div class="pill" role="status"><span class="spin"></span>Reconnecting{reason ? ` · ${reason}` : ""}</div>
  {/if}
</div>

<style>
  .feed {
    position: absolute;
    inset: 0;
  }
  canvas,
  img {
    display: block;
    width: 100%;
    height: 100%;
    object-fit: cover;
    transition:
      opacity var(--t-med),
      filter var(--t-med);
  }
  .contain img {
    object-fit: contain;
  }
  .dim {
    opacity: 0.45;
    filter: grayscale(0.7);
  }
  .note {
    position: absolute;
    inset: 0;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 6px;
    font-size: 12.5px;
    color: var(--fg-faint);
    text-align: center;
    padding: 12px;
  }
  .why {
    font-size: 11.5px;
    opacity: 0.8;
  }
  .pill {
    position: absolute;
    left: 50%;
    bottom: 10px;
    transform: translateX(-50%);
    display: flex;
    align-items: center;
    gap: 7px;
    padding: 4px 10px;
    border-radius: var(--r-pill);
    font-size: 11.5px;
    color: var(--fg);
    background: rgba(0, 0, 0, 0.7);
    white-space: nowrap;
  }
  .spin {
    width: 10px;
    height: 10px;
    border-radius: 50%;
    border: 2px solid currentColor;
    border-right-color: transparent;
    animation: spin 0.9s linear infinite;
  }
  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }
</style>
