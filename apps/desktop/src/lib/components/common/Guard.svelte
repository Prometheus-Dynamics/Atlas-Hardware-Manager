<script lang="ts">
  // Keeps a part that throws while it renders to itself: it shows what went
  // wrong in its place, with a retry, and the rest of the page keeps working.
  // The error, with its stack, goes to atlas.log.
  import type { Snippet } from "svelte";
  import { errorText, reportUiError } from "#lib/api/client.ts";
  import { page } from "$app/state";
  import Button from "./Button.svelte";

  let { name, children }: { name: string; children: Snippet } = $props();

  function report(error: unknown) {
    void reportUiError(error, `${page.url.pathname}${page.url.search} · ${name}`);
  }
</script>

<svelte:boundary onerror={report}>
  {@render children()}
  {#snippet failed(error, reset)}
    <div class="failed" role="alert">
      <p><b>{name}</b> hit a bug: {errorText(error)}. It's in atlas.log.</p>
      <Button size="sm" variant="ghost" icon="refresh" onclick={reset}>Retry</Button>
    </div>
  {/snippet}
</svelte:boundary>

<style>
  .failed {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 8px 12px;
    border: 1px solid color-mix(in srgb, var(--color-warn-fg) 35%, transparent);
    border-radius: 6px;
    background: color-mix(in srgb, var(--color-warn-fg) 8%, transparent);
    font-size: 12px;
    color: var(--color-fg-muted);
  }
  .failed p {
    flex: 1;
    min-width: 0;
  }
</style>
