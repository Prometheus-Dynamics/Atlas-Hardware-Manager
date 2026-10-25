<script lang="ts">
  // Asks before something that takes the board offline or deletes from it,
  // naming the device. Only what can't be undone (`typed`: removing a slot's
  // image) asks for the end of its serial too, and only in ROBOT mode; a
  // restart, a power-off or forgetting the device is one click to confirm.
  import type { DeviceRecord } from "#lib/api/client.ts";
  import Button from "#lib/components/common/Button.svelte";
  import { ui } from "#lib/stores/ui.svelte.ts";
  import { portal } from "#lib/ui/portal.ts";

  let {
    record,
    title,
    body,
    label,
    busy = false,
    typed: needsTyping = false,
    onconfirm,
    oncancel,
  }: {
    record: DeviceRecord;
    title: string;
    body: string;
    /** The confirm button's label. */
    label: string;
    busy?: boolean;
    /** It can't be undone: in ROBOT mode, typing the serial's end confirms. */
    typed?: boolean;
    onconfirm: () => void;
    oncancel: () => void;
  } = $props();

  let typed = $state("");
  const tail = $derived(record.key.serial.slice(-4));
  const robot = $derived(ui.mode === "robot" && needsTyping);
  const confirmed = $derived(!robot || typed.trim().toLowerCase() === tail.toLowerCase());
  function confirm() {
    if (confirmed && !busy) onconfirm();
  }
</script>

<svelte:window onkeydown={(e) => e.key === "Escape" && oncancel()} />

<div class="modal" use:portal>
  <button type="button" class="backdrop" aria-label="Cancel" onclick={oncancel}></button>
  <div class="dialog panel" role="dialog" aria-modal="true" aria-label={title}>
    <h2 class="text-[14px] font-semibold text-fg">{title}</h2>
    <p class="text-[12.5px] text-fg-muted">{body}</p>
    {#if robot}
      <label class="flex flex-col gap-1 text-[12px] text-warn-fg">
        ROBOT mode: type {tail} to confirm.
        <!-- autofocus is ignored while the opening button has focus, and the
             portal's move drops focus: take it once the dialog is in place. -->
        <input class="input mono" bind:value={typed} placeholder={tail} {@attach (el) => void requestAnimationFrame(() => el.focus())} onkeydown={(e) => e.key === "Enter" && confirm()} />
      </label>
    {/if}
    <div class="flex justify-end gap-2">
      <Button size="sm" variant="ghost" onclick={oncancel}>Cancel</Button>
      <Button size="sm" variant="danger" disabled={!confirmed} {busy} onclick={confirm}>{label}</Button>
    </div>
  </div>
</div>

<style>
  .modal {
    position: fixed;
    inset: 0;
    z-index: 95;
    display: grid;
    place-items: center;
  }
  .backdrop {
    position: absolute;
    inset: 0;
    background: rgba(5, 6, 8, 0.65);
    cursor: default;
  }
  .dialog {
    position: relative;
    width: min(400px, calc(100vw - 32px));
    display: flex;
    flex-direction: column;
    gap: 10px;
    padding: 14px;
  }
</style>
