<script lang="ts">
  import { api, openExternal, type ReleaseEntry } from "#lib/api/client.ts";
  import Button from "#lib/components/common/Button.svelte";
  import ConfirmButton from "#lib/components/common/ConfirmButton.svelte";
  import Icon from "#lib/components/common/Icon.svelte";
  import Pill from "#lib/components/common/Pill.svelte";
  import ProgressBar from "#lib/components/common/ProgressBar.svelte";
  import { bytes, deviceName, primaryVersion, type Tone } from "#lib/format.ts";
  import { devices } from "#lib/stores/devices.svelte.ts";
  import { releases } from "#lib/stores/releases.svelte.ts";
  import { toasts } from "#lib/stores/toasts.svelte.ts";

  let { entry }: { entry: ReleaseEntry } = $props();

  const progress = $derived(releases.downloads.get(entry.id));
  // The devices of its family that run it now.
  const running = $derived(devices.all.filter((d) => d.key.family === entry.family && primaryVersion(d.identity) === entry.version));
  const channel: Record<ReleaseEntry["channel"], { label: string; tone: Tone }> = {
    stable: { label: "Stable", tone: "success" },
    beta: { label: "Beta", tone: "info" },
    local: { label: "Local file", tone: "neutral" },
  };

  // A local file's signature: who signed it, and what to be wary of.
  const signature = $derived.by(() => {
    const status = entry.signature;
    if (!status) return entry.signed ? { tone: "success" as Tone, label: "Signed", title: "Its manifest is signed by a trusted key", by: "" } : null;
    switch (status.state) {
      case "signed": {
        const key = status.key;
        const purpose = key.purpose === "dev" ? "dev/example" : "production";
        const by = `Signed by ${key.name} (${key.owner}, ${purpose})`;
        if (status.expired) return { tone: "warning" as Tone, label: "Key expired", title: `${by}; the key expired on ${key.expires}`, by };
        if (key.purpose === "dev") return { tone: "warning" as Tone, label: "Dev key", title: `${by}: fine for testing, not a production release`, by };
        return { tone: "success" as Tone, label: "Signed", title: `${by} · ${key.contact}`, by };
      }
      case "unknown-key":
        return { tone: "warning" as Tone, label: "Unknown key", title: `Signed by key minisign:${status.key_id}, which isn't in Atlas's trusted keys`, by: "" };
      case "bad":
        return { tone: "error" as Tone, label: "Bad signature", title: `Its signature doesn't check out (${status.reason}): the file may have changed after it was signed`, by: "" };
      default:
        return null;
    }
  });

  async function togglePin() {
    await api.setReleasePinned(entry.id, !entry.pinned);
    await releases.load();
  }

  async function remove() {
    await api.removeRelease(entry.id);
    await releases.load();
    toasts.success(`Removed ${entry.family} ${entry.version}.`);
  }
</script>

<div class="row">
  <div class="main min-w-0">
    <p class="mono truncate text-[13px] font-medium text-fg" title={entry.version}>{entry.version}</p>
    <p class="truncate text-[12px] text-fg-faint" title={entry.path ?? entry.artifact_name}>
      <span class="mono">{entry.artifact_name}</span> · {entry.origin.kind === "remote" ? `from ${entry.origin.source}` : "added by you"}{entry.boards.length ? ` · ${entry.boards.join(", ")}` : ""}
    </p>
    {#if signature?.by}
      <p class="truncate text-[12px] text-fg-faint" title={signature.title}>
        <Icon name="shield-check" size={12} class="mr-1 inline align-[-1px]" />{signature.by}
      </p>
    {/if}
    {#if running.length}
      <p class="truncate text-[12px] text-ok-fg" title={running.map(deviceName).join(", ")}>
        <Icon name="circle-check" size={12} class="mr-1 inline align-[-1px]" />Runs on {running.map(deviceName).join(", ")}
      </p>
    {/if}
  </div>
  <div class="pills flex flex-wrap items-center gap-1.5">
    <Pill tone={channel[entry.channel].tone} label={channel[entry.channel].label} />
    {#if signature}
      <Pill tone={signature.tone} icon={signature.tone === "success" ? "shield-check" : "alert-triangle"} label={signature.label} title={signature.title} />
    {:else}
      <Pill tone="warning" icon="alert-triangle" label="Unsigned" title="No signature: where this file came from can't be verified" />
    {/if}
  </div>
  <span class="size text-right text-[12.5px] tabular-nums text-fg-muted">{bytes(entry.size_bytes)}</span>
  <div class="state flex min-w-0 items-center justify-end">
    {#if progress}
      <div class="flex w-full flex-col gap-1">
        <ProgressBar value={progress.total ? progress.downloaded / progress.total : null} label="Download progress" size={5} />
        <span class="text-[11.5px] tabular-nums text-fg-faint">
          {bytes(progress.downloaded)}{progress.total ? ` of ${bytes(progress.total)}` : ""}
        </span>
      </div>
    {:else if entry.path}
      <span class="flex items-center gap-1.5 whitespace-nowrap text-[12.5px] text-ok-fg"><Icon name="device-sd-card" size={15} />On this computer</span>
    {:else}
      <Button size="sm" icon="download" onclick={() => releases.download(entry.id)}>Download</Button>
    {/if}
  </div>
  <div class="tools flex items-center justify-end gap-1">
    {#if entry.notes_url}
      <Button variant="ghost" size="sm" iconRight="external-link" onclick={() => openExternal(entry.notes_url!)}>Notes</Button>
    {/if}
    {#if entry.channel === "local"}
      <Button
        variant="ghost"
        size="sm"
        icon={entry.pinned ? "pinned" : "pin"}
        label={entry.pinned ? "Unpin" : "Pin"}
        title={entry.pinned ? "Pinned: stays in the list" : "Pin to keep it; unpinned images drop off after the 5 newest"}
        action={togglePin}
      />
    {/if}
    <ConfirmButton action={remove} size="sm" variant="ghost" icon="trash" label="Remove {entry.version}" prompt="Remove?" confirmLabel="Remove" />
  </div>
</div>

<style>
  .row {
    display: grid;
    grid-template-columns: minmax(200px, 1.6fr) minmax(170px, 1fr) 70px minmax(140px, 0.8fr) minmax(110px, auto);
    align-items: center;
    gap: 16px;
    padding: 10px 14px;
    border-radius: var(--r-card);
    transition: background var(--t-fast);
  }
  .row:hover {
    background: var(--glass);
  }
  /* In a narrower family card: two lines. What it is and where it stands on
     top; channel, signature, size and the row's tools below. */
  @container (max-width: 760px) {
    .row {
      grid-template-columns: minmax(0, 1fr) auto auto;
      grid-template-areas:
        "main main state"
        "pills size tools";
      gap: 6px 12px;
      padding: 10px 12px;
    }
    .main {
      grid-area: main;
    }
    .pills {
      grid-area: pills;
    }
    .size {
      grid-area: size;
    }
    .state {
      grid-area: state;
    }
    .tools {
      grid-area: tools;
    }
  }
</style>
