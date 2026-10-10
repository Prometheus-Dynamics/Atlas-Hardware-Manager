<script lang="ts">
  import { flip } from "svelte/animate";
  import Icon from "#lib/components/common/Icon.svelte";
  import { toasts, type ToastTone } from "#lib/stores/toasts.svelte.ts";
  import type { IconName } from "#lib/ui/icons.ts";
  import { DUR, ease, ms, slideIn, softFade } from "#lib/ui/motion.ts";

  const icon: Record<ToastTone, IconName> = {
    success: "circle-check",
    error: "alert-circle",
    warning: "alert-triangle",
    info: "info-circle",
  };
</script>

<div class="stack" aria-live="polite">
  {#each toasts.items as toast (toast.id)}
    <div
      class="toast glass-layer {toast.tone}"
      role={toast.tone === "error" ? "alert" : "status"}
      animate:flip={{ duration: ms(DUR.enter), easing: ease }}
      in:slideIn={{ x: 24, duration: 220 }}
      out:softFade
    >
      <span class="ic"><Icon name={icon[toast.tone]} size={16} stroke={2} /></span>
      <p class="flex-1 text-[13px] leading-snug text-fg">{toast.message}</p>
      <button type="button" class="close" aria-label="Dismiss" onclick={() => toasts.dismiss(toast.id)}>
        <Icon name="x" size={14} />
      </button>
    </div>
  {/each}
</div>

<style>
  .stack {
    position: absolute;
    right: 20px;
    bottom: 16px;
    z-index: 50;
    display: flex;
    flex-direction: column;
    gap: 8px;
    width: 360px;
    max-width: calc(100vw - 40px);
    pointer-events: none;
  }
  .toast {
    pointer-events: auto;
    display: flex;
    align-items: flex-start;
    gap: 10px;
    padding: 11px 12px;
    border-radius: var(--r-card);
  }
  .ic {
    display: inline-flex;
    margin-top: 1px;
  }
  .success .ic {
    color: var(--ok-fg);
  }
  .error .ic {
    color: var(--err-fg);
  }
  .warning .ic {
    color: var(--warn-fg);
  }
  .info .ic {
    color: var(--info-fg);
  }
  .close {
    display: inline-flex;
    padding: 2px;
    border-radius: var(--r-md);
    color: var(--fg-faint);
  }
  .close:hover {
    color: var(--fg);
    background: var(--glass-hover);
  }
</style>
