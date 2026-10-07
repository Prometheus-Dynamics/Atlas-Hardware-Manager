<script lang="ts">
  // What runs on the board now: OS, device package, and the A/B state when
  // the board reports it. Only what exists.
  import type { DeviceRecord } from "#lib/api/client.ts";
  import IconTile from "#lib/components/common/IconTile.svelte";
  import { primaryVersion } from "#lib/format.ts";
  import { isRecovery, storageName } from "#lib/present.ts";
  import { slotFacts } from "./software";

  let { record }: { record: DeviceRecord } = $props();

  const attrs = $derived(record.identity.attributes ?? {});
  const recovery = $derived(isRecovery(record));
  const os = $derived.by(() => {
    if (recovery) return "Unknown while in USB boot";
    const version = attrs.os_version ?? primaryVersion(record.identity);
    if (attrs.os) return version ? `${attrs.os} ${version}` : attrs.os;
    return version ?? "Unknown";
  });
  const facts = $derived(
    [
      { label: "Device package", value: record.identity.versions.device_package ?? null },
      ...(recovery ? [] : slotFacts(record)),
      { label: "Storage", value: recovery ? storageName(attrs.storage) : null },
    ].filter((f): f is { label: string; value: string } => !!f.value),
  );
</script>

<div class="glass flex items-center gap-3 px-4 py-3">
  <IconTile icon={recovery ? "usb" : "package"} size={34} tone={recovery ? "accent" : "neutral"} />
  <div class="min-w-0 flex-1">
    <p class="truncate text-[13.5px] font-medium text-fg" class:mono={!recovery && !attrs.os}>{os}</p>
    {#if facts.length > 0}
      <p class="truncate text-[12px] text-fg-faint">
        {#each facts as fact, i (fact.label)}{i > 0 ? " · " : ""}{fact.label}
          <span class="text-fg-muted">{fact.value}</span>{/each}
      </p>
    {/if}
  </div>
</div>
