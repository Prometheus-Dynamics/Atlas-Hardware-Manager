<script lang="ts">
  // What Atlas checks for the chosen image, in plain words.
  import type { ReleaseEntry } from "$lib/api/client";
  import Icon from "$lib/components/common/Icon.svelte";
  import { bytes } from "$lib/format";
  import type { IconName } from "$lib/ui/icons";

  let { entry }: { entry: ReleaseEntry } = $props();

  const checks = $derived.by(() => {
    const list: { icon: IconName; tone: "ok" | "warn" | "info"; text: string }[] = [];
    if (entry.signed) {
      const by = entry.origin.kind === "remote" ? ` by ${entry.origin.source}` : "";
      list.push({ icon: "shield-check", tone: "ok", text: `Signed${by}` });
    } else {
      list.push({ icon: "alert-triangle", tone: "warn", text: "Unsigned — Atlas will warn but won't stop you" });
    }
    if (!entry.path) list.push({ icon: "cloud-download", tone: "info", text: `Downloads first (${bytes(entry.size_bytes)})` });
    list.push({ icon: "checks", tone: "ok", text: "Read back and checked after writing" });
    return list;
  });
</script>

<ul class="glass flex flex-col gap-2 px-4 py-3">
  {#each checks as check (check.text)}
    <li class="flex items-center gap-2.5 text-[13px] text-fg">
      <span class="ic {check.tone}"><Icon name={check.icon} size={14} stroke={2} /></span>
      {check.text}
    </li>
  {/each}
</ul>

<style>
  .ic {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 22px;
    height: 22px;
    border-radius: 50%;
    flex-shrink: 0;
  }
  .ok {
    background: var(--ok-bg);
    color: var(--ok-fg);
  }
  .warn {
    background: var(--warn-bg);
    color: var(--warn-fg);
  }
  .info {
    background: var(--info-bg);
    color: var(--info-fg);
  }
</style>
