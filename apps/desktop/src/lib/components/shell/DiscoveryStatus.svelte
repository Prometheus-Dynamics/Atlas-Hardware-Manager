<script lang="ts">
  // "Live": Atlas is watching for devices and shows changes as they happen,
  // so there is no refresh to press. Clicking still forces a look.
  import { devices } from "#lib/stores/devices.svelte.ts";

  const status = $derived(devices.discovery);
  const live = $derived(status?.live ?? false);
  const sources = $derived(status ? [...status.watching, ...status.polled] : []);
  const tip = $derived(
    live
      ? `Watching ${sources.join(" and ") || "for devices"}. Changes show up as they happen. Click to look again now.`
      : "Not watching for devices (turned off in Settings). Click to look now.",
  );
</script>

<button type="button" class="status" class:live class:busy={devices.scanning} title={tip} onclick={() => devices.scanNow()}>
  <span class="beacon"><span class="core"></span></span>
  {live ? "Live" : "Paused"}
</button>

<style>
  .status {
    display: inline-flex;
    align-items: center;
    gap: 8px;
    height: 32px;
    padding: 0 12px 0 10px;
    border-radius: var(--r-pill);
    font-size: 12.5px;
    font-weight: 500;
    color: var(--fg-muted);
    border: 1px solid var(--glass-border);
    background: var(--glass);
    transition:
      background var(--t-fast),
      color var(--t-fast);
  }
  .status:hover {
    color: var(--fg);
    background: var(--glass-hover);
  }
  .beacon {
    position: relative;
    display: inline-flex;
    width: 8px;
    height: 8px;
  }
  .core {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: var(--offline);
    transition: background var(--t-med);
  }
  .live .core {
    background: var(--ok);
  }
  /* One quick ping while actually looking; still otherwise. */
  .live.busy .beacon::after {
    content: "";
    position: absolute;
    inset: 0;
    border-radius: 50%;
    border: 1.5px solid var(--ok);
    animation: ping 0.9s var(--ease-out) infinite;
  }
  @keyframes ping {
    from {
      transform: scale(1);
      opacity: 0.7;
    }
    to {
      transform: scale(3);
      opacity: 0;
    }
  }
</style>
