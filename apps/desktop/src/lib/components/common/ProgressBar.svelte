<script lang="ts">
  // A rounded progress bar whose width animates; null shows a moving sheen.
  let {
    value,
    tone = "primary",
    size = 6,
    label,
  }: {
    value: number | null;
    tone?: "primary" | "success" | "warning" | "error" | "neutral";
    size?: number;
    label?: string;
  } = $props();

  const pct = $derived(value === null ? null : Math.max(0, Math.min(100, value * 100)));
</script>

<div
  class="track"
  style="--h: {size}px"
  role="progressbar"
  aria-label={label}
  aria-valuemin="0"
  aria-valuemax="100"
  aria-valuenow={pct === null ? undefined : Math.round(pct)}
>
  {#if pct === null}
    <div class="fill indeterminate {tone}"></div>
  {:else}
    <div class="fill {tone}" style="width: {pct}%"></div>
  {/if}
</div>

<style>
  .track {
    position: relative;
    width: 100%;
    height: var(--h);
    border-radius: var(--r-pill);
    background: var(--glass-strong);
    overflow: hidden;
  }
  .fill {
    height: 100%;
    border-radius: inherit;
    transition: width var(--t-data) var(--ease-out);
  }
  .primary {
    background: linear-gradient(90deg, var(--accent-press), var(--accent-hover));
    box-shadow: 0 0 10px var(--accent-glow);
  }
  .success {
    background: var(--ok);
  }
  .warning {
    background: var(--warn);
  }
  .error {
    background: var(--err);
  }
  .neutral {
    background: var(--fg-faint);
  }
  .indeterminate {
    width: 35%;
    animation: slide 1.3s var(--ease-out) infinite;
  }
  @keyframes slide {
    from {
      transform: translateX(-100%);
    }
    to {
      transform: translateX(300%);
    }
  }
</style>
