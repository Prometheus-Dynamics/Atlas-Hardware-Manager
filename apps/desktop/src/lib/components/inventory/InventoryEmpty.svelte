<script lang="ts">
  import EmptyState from "$lib/components/common/EmptyState.svelte";
  import { devices } from "$lib/stores/devices.svelte";
</script>

<EmptyState icon="fa-plug" title={devices.scanning ? "Looking for devices…" : "No devices found yet"}>
  <p>
    Connect the robot with a USB cable or Ethernet, or plug a device in directly. Atlas scans every few seconds and
    devices appear here as they answer, including boards behind a gateway and Raspberry Pi modules in USB boot mode.
  </p>
  <p class="mt-2">
    No hardware at hand? Turn on <a href="/settings" class="underline">simulated devices</a> in Settings.
  </p>
  <button type="button" class="btn btn-sm preset-tonal mt-3" onclick={() => devices.scanNow()} disabled={devices.scanning}>
    <i class="fa-solid fa-satellite-dish" aria-hidden="true"></i>Scan now
  </button>
</EmptyState>
