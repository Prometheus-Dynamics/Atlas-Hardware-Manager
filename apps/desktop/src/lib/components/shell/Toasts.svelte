<script lang="ts">
  import { toasts, type ToastTone } from "$lib/stores/toasts.svelte";

  const style: Record<ToastTone, string> = {
    success: "border-success-600/70 text-success-200",
    error: "border-error-500/70 text-error-100",
    warning: "border-warning-600/70 text-warning-200",
    info: "border-secondary-500/70 text-secondary-100",
  };
  const icon: Record<ToastTone, string> = {
    success: "fa-circle-check",
    error: "fa-circle-exclamation",
    warning: "fa-triangle-exclamation",
    info: "fa-circle-info",
  };
</script>

<div class="pointer-events-none fixed right-4 top-[5.75rem] z-50 flex w-96 max-w-[calc(100vw-2rem)] flex-col gap-2" aria-live="polite">
  {#each toasts.items as toast (toast.id)}
    <div
      class="pointer-events-auto flex items-start gap-2 rounded-container border bg-surface-900 px-3 py-2 text-xs {style[toast.tone]}"
      role={toast.tone === "error" ? "alert" : "status"}
    >
      <i class="fa-solid {icon[toast.tone]} mt-0.5" aria-hidden="true"></i>
      <p class="flex-1 text-surface-100">{toast.message}</p>
      <button type="button" class="text-surface-400 hover:text-surface-50" aria-label="Dismiss" onclick={() => toasts.dismiss(toast.id)}>
        <i class="fa-solid fa-xmark" aria-hidden="true"></i>
      </button>
    </div>
  {/each}
</div>
