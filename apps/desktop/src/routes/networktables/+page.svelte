<script lang="ts">
  import PageHeader from "#lib/components/common/PageHeader.svelte";
  import SegmentedControl from "#lib/components/common/SegmentedControl.svelte";
  import Page from "#lib/components/layout/Page.svelte";
  import NtServer from "#lib/components/nt/NtServer.svelte";
  import NtViewer from "#lib/components/nt/NtViewer.svelte";

  const TAB_KEY = "atlas.nt.tab";
  let tab = $state((() => {
    try {
      return localStorage.getItem(TAB_KEY) === "server" ? "server" : "viewer";
    } catch {
      return "viewer";
    }
  })());
  $effect(() => {
    try {
      localStorage.setItem(TAB_KEY, tab);
    } catch {
      // Private mode: the tab lasts this session.
    }
  });
  const tabs = [
    { value: "viewer", label: "Viewer", icon: "eye" as const },
    { value: "server", label: "Local server", icon: "router" as const },
  ];
</script>

<svelte:head><title>NetworkTables · Atlas</title></svelte:head>

<Page>
  {#snippet header()}
    <PageHeader title="NetworkTables" subtitle="See a robot's or a camera's NetworkTables live, or run a server here to test a camera without a robot.">
      {#snippet actions()}
        <SegmentedControl options={tabs} bind:value={tab} label="NetworkTables view" />
      {/snippet}
    </PageHeader>
  {/snippet}

  <!-- Both stay mounted: switching tabs keeps the viewer connected. -->
  <div class="flex min-h-0 flex-1 flex-col" class:hidden={tab !== "viewer"}><NtViewer /></div>
  <div class="flex min-h-0 flex-1 flex-col overflow-y-auto" class:hidden={tab !== "server"}><NtServer /></div>
</Page>
