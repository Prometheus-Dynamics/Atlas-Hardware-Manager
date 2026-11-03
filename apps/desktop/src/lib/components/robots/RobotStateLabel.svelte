<script lang="ts">
  // A robot's state as words with an icon, not a chip: it sits beside
  // buttons and mustn't look like one. "Devices missing" shows once it has
  // lasted a few seconds, so one missed reading doesn't flip it.
  import type { RobotState } from "#lib/api/client.ts";
  import Icon from "#lib/components/common/Icon.svelte";
  import { ROBOT_STATE } from "./robotState";

  const MISSING_HOLD_MS = 8_000;

  let { value }: { value: RobotState } = $props();

  // svelte-ignore state_referenced_locally
  let shown = $state<RobotState>(value);
  $effect(() => {
    const next = value;
    if (next !== "missing-devices") {
      shown = next;
      return;
    }
    const timer = setTimeout(() => (shown = next), MISSING_HOLD_MS);
    return () => clearTimeout(timer);
  });
  const look = $derived(ROBOT_STATE[shown]);
</script>

<span class="state {look.tone}"><Icon name={look.icon} size={14} stroke={2} />{look.label}</span>

<style>
  .state {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    font-size: 12.5px;
    font-weight: 500;
    white-space: nowrap;
  }
  .success {
    color: var(--ok-fg);
  }
  .warning {
    color: var(--warn-fg);
  }
  .error {
    color: var(--err-fg);
  }
  .neutral,
  .primary,
  .info {
    color: var(--fg-muted);
  }
</style>
