<script lang="ts">
  // Whether Atlas is watching for devices, and a Rescan that is always there:
  // it looks for devices on every link now and re-reads what they report.
  // The time since the last scan (Atlas's own or this one) says how fresh
  // the data is; a spinner shows while one runs.
  import Icon from "#lib/components/common/Icon.svelte";
  import { timeAgo } from "#lib/format.ts";
  import { clock } from "#lib/stores/clock.svelte.ts";
  import { devices } from "#lib/stores/devices.svelte.ts";

  const status = $derived(devices.discovery);
  const live = $derived(status?.live ?? false);
  const sources = $derived(status ? [...status.watching, ...status.polled] : []);
  const when = $derived(devices.scanning ? "scanning" : devices.lastScanAt === null ? "not yet" : timeAgo(devices.lastScanAt, clock.now));
  const tip = $derived(
    (live
      ? `Watching ${sources.join(" and ") || "for devices"}; changes show up as they happen.`
      : "Not watching for devices (turned off in Settings).") + " Rescan looks on every link now and re-reads each device.",
  );
</script>

<div class="status" class:live title={tip}>
  <span class="state"><span class="core"></span>{live ? "Live" : "Paused"}</span>
  <button type="button" class="rescan" disabled={devices.scanning} aria-busy={devices.scanning} onclick={() => devices.scanNow()}>
    <Icon name={devices.scanning ? "loader-2" : "refresh"} class={devices.scanning ? "spin" : ""} size={14} />
    Rescan
    <span class="when">{when}</span>
  </button>
</div>

<style>
  .status {
    display: inline-flex;
    align-items: stretch;
    height: 32px;
    border-radius: var(--r-pill);
    font-size: 12.5px;
    font-weight: 500;
    color: var(--fg-muted);
    border: 1px solid var(--glass-border);
    background: var(--glass);
    overflow: hidden;
  }
  .state {
    display: inline-flex;
    align-items: center;
    gap: 7px;
    padding: 0 10px;
    border-right: 1px solid var(--glass-border);
  }
  .core {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: var(--offline);
  }
  .live .core {
    background: var(--ok);
  }
  .rescan {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 0 12px 0 10px;
    font: inherit;
    color: var(--fg);
    background: none;
    border: 0;
    cursor: pointer;
    transition: background var(--t-fast);
  }
  .rescan:hover:not(:disabled) {
    background: var(--glass-hover);
  }
  .rescan:disabled {
    cursor: progress;
  }
  .when {
    color: var(--fg-muted);
    font-variant-numeric: tabular-nums;
    min-width: 6ch;
  }
</style>
