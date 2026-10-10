<script lang="ts">
  // An on/off switch (a checkbox underneath, so forms and labels work).
  let {
    checked = $bindable(false),
    label,
    disabled = false,
  }: { checked?: boolean; label?: string; disabled?: boolean } = $props();
</script>

<input type="checkbox" class="toggle" role="switch" bind:checked {disabled} aria-label={label} />

<style>
  .toggle {
    appearance: none;
    position: relative;
    width: 36px;
    height: 20px;
    border-radius: var(--r-pill);
    background: var(--glass-strong);
    border: 1px solid var(--glass-border);
    cursor: pointer;
    transition: background var(--t-med);
    flex-shrink: 0;
  }
  .toggle::after {
    content: "";
    position: absolute;
    top: 2px;
    left: 2px;
    width: 15px;
    height: 15px;
    border-radius: var(--r-sm);
    background: var(--fg);
    box-shadow: 0 1px 3px rgba(0, 0, 0, 0.4);
    transition: transform var(--t-med) var(--ease-out);
  }
  .toggle:checked {
    background: var(--accent);
    border-color: transparent;
  }
  .toggle:checked::after {
    transform: translateX(16px);
  }
  .toggle:disabled {
    opacity: 0.45;
    cursor: default;
  }
</style>
