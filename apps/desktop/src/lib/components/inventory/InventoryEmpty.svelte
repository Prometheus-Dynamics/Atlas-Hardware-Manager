<script lang="ts">
  import Button from "#lib/components/common/Button.svelte";
  import EmptyState from "#lib/components/common/EmptyState.svelte";
  import { devices } from "#lib/stores/devices.svelte.ts";
</script>

<EmptyState icon="plug-connected" title={devices.scanning ? "Looking for devices…" : "No devices yet"}>
  <p>
    Connect the robot with a USB cable or Ethernet, or plug a device in directly. Devices appear here as they answer,
    including boards behind a gateway and Raspberry Pi modules in USB boot.
  </p>
  <p class="mt-2">No hardware at hand? Turn on <a href="/settings" class="link">simulated devices</a> in Settings.</p>
  <div class="mt-4 flex justify-center">
    <Button icon="radar-2" busy={devices.scanning} onclick={() => devices.scanNow()}>Scan now</Button>
  </div>
</EmptyState>
