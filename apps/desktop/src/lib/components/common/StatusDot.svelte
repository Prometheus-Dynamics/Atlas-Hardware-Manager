<script lang="ts" module>
  export type DotState = "online" | "busy" | "failed" | "offline";
</script>

<script lang="ts">
  // An 8px state dot: online, busy (accent, pulsing), failed, offline.

  let { state, label }: { state: DotState; label?: string } = $props();
</script>

<span class="dot {state}" role="img" aria-label={label ?? state} title={label ?? state}></span>

<style>
  .dot {
    position: relative;
    display: inline-block;
    width: 8px;
    height: 8px;
    border-radius: 50%;
    flex-shrink: 0;
    transition: background var(--t-med);
  }
  .online {
    background: var(--ok);
    box-shadow: 0 0 0 3px var(--ok-bg);
  }
  .busy {
    background: var(--accent);
    box-shadow: 0 0 0 3px var(--accent-tint);
  }
  .busy::after {
    content: "";
    position: absolute;
    inset: 0;
    border-radius: 50%;
    background: var(--accent);
    animation: ping 1.6s var(--ease-out) infinite;
  }
  .failed {
    background: var(--err);
    box-shadow: 0 0 0 3px var(--err-bg);
  }
  .offline {
    background: var(--offline);
  }
  @keyframes ping {
    from {
      transform: scale(1);
      opacity: 0.6;
    }
    to {
      transform: scale(2.6);
      opacity: 0;
    }
  }
</style>
