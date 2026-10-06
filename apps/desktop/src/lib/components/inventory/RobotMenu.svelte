<script lang="ts">
  // The inventory title doubles as the robot filter: "All devices ▾".
  import Icon from "#lib/components/common/Icon.svelte";
  import { robots } from "#lib/stores/robots.svelte.ts";
  import { NO_ROBOT, ui } from "#lib/stores/ui.svelte.ts";
  import { popover } from "#lib/ui/motion.ts";

  let open = $state(false);
  let root: HTMLElement | undefined = $state();

  const title = $derived(ui.robot === null ? "All devices" : ui.robot === NO_ROBOT ? "Not on a robot" : ui.robot);
  const options = $derived([
    { value: null, label: "All devices" },
    ...robots.names.map((name) => ({ value: name as string | null, label: name })),
    { value: NO_ROBOT, label: "Not on a robot" },
  ]);

  function choose(value: string | null) {
    ui.robot = value;
    open = false;
  }

  function onwindowclick(event: MouseEvent) {
    if (open && root && !root.contains(event.target as Node)) open = false;
  }
</script>

<svelte:window onclick={onwindowclick} />

<div class="relative" bind:this={root}>
  <button type="button" class="title" aria-haspopup="listbox" aria-expanded={open} onclick={() => (open = !open)}>
    {#if ui.robot !== null && ui.robot !== NO_ROBOT}<Icon name="robot" size={22} stroke={1.6} class="text-fg-muted" />{/if}
    <span class="truncate">{title}</span>
    <span class="chev" class:open><Icon name="chevron-down" size={18} /></span>
  </button>
  {#if open}
    <ul class="menu glass-layer" role="listbox" aria-label="Show devices for" transition:popover>
      {#each options as option (option.value ?? "all")}
        <li>
          <button type="button" role="option" aria-selected={ui.robot === option.value} onclick={() => choose(option.value)}>
            <Icon name={option.value === null ? "layout-grid" : option.value === NO_ROBOT ? "circle-dashed" : "robot"} size={16} />
            <span class="flex-1 truncate text-left">{option.label}</span>
            {#if ui.robot === option.value}<Icon name="check" size={15} class="text-accent-text" />{/if}
          </button>
        </li>
      {/each}
    </ul>
  {/if}
</div>

<style>
  .title {
    display: inline-flex;
    align-items: center;
    gap: 8px;
    max-width: 100%;
    margin-left: -8px;
    padding: 2px 8px;
    border-radius: 10px;
    font-size: 22px;
    font-weight: 600;
    letter-spacing: -0.01em;
    color: var(--fg);
    transition: background var(--t-fast);
  }
  .title:hover {
    background: var(--glass);
  }
  .chev {
    display: inline-flex;
    color: var(--fg-faint);
    transition: transform var(--t-med) var(--ease-out);
  }
  .chev.open {
    transform: rotate(180deg);
  }
  .menu {
    position: absolute;
    top: calc(100% + 6px);
    left: -8px;
    z-index: 35;
    min-width: 220px;
    padding: 6px;
    border-radius: 14px;
    transform-origin: top left;
  }
  .menu button {
    display: flex;
    align-items: center;
    gap: 10px;
    width: 100%;
    padding: 8px 10px;
    border-radius: 9px;
    font-size: 13px;
    color: var(--fg);
  }
  .menu button:hover {
    background: var(--glass-hover);
  }
</style>
