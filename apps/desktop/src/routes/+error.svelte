<script lang="ts">
  // What a page that failed shows instead of SvelteKit's bare "500 Internal
  // Error": which page, what went wrong, and a way back. hooks.client.ts has
  // already put the error, with its stack, in atlas.log.
  import { goto } from "$app/navigation";
  import { page } from "$app/state";
  import Button from "#lib/components/common/Button.svelte";
  import EmptyState from "#lib/components/common/EmptyState.svelte";

  const missing = $derived(page.status === 404);
</script>

<svelte:head><title>{missing ? "Not found" : "Something went wrong"} · Atlas</title></svelte:head>

<div class="flex h-full items-center justify-center">
  <EmptyState icon="alert-triangle" title={missing ? "There's no page here" : "This page hit a bug"}>
    {#if missing}
      <p>{page.url.pathname} isn't a page in Atlas.</p>
    {:else}
      <p>
        Atlas couldn't show {page.url.pathname}: {page.error?.message ?? "an unknown error"}. It's written to atlas.log in
        Atlas's data folder; Settings has the folder.
      </p>
    {/if}
    <div class="mt-4 flex justify-center gap-2">
      <Button icon="refresh" onclick={() => location.reload()}>Try again</Button>
      <Button variant="ghost" onclick={() => goto("/")}>Overview</Button>
    </div>
  </EmptyState>
</div>
