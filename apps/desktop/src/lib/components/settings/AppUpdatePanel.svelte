<script lang="ts">
  // Atlas's own updates: this version, a newer release when there is one,
  // and installing it (download, check, install, restart). Whether Atlas
  // looks by itself is in Preferences.
  import { openExternal } from "#lib/api/client.ts";
  import Button from "#lib/components/common/Button.svelte";
  import GlassCard from "#lib/components/common/GlassCard.svelte";
  import Icon from "#lib/components/common/Icon.svelte";
  import Pill from "#lib/components/common/Pill.svelte";
  import ProgressBar from "#lib/components/common/ProgressBar.svelte";
  import { bytes, timeAgo } from "#lib/format.ts";
  import { appUpdate } from "#lib/stores/appUpdate.svelte.ts";

  const KINDS: Record<string, string> = {
    rpm: "installed from its .rpm",
    deb: "installed from its .deb",
    "app-image": "running as an AppImage",
    nsis: "installed with its setup program",
    msi: "installed from its .msi",
    dmg: "installed from its disk image",
    source: "built from source",
  };

  const s = $derived(appUpdate.status);
  const release = $derived(s?.available ?? null);
  /** It can replace itself: installed from a package Atlas knows. */
  const replaces = $derived(!!s && s.kind !== "source" && s.kind !== null && !!release?.installer);
  const subtitle = $derived(
    s ? `${s.release_build ? `Version ${s.current}` : `Development build (${s.current})`} · ${KINDS[s.kind ?? "source"]}` : undefined,
  );
  const notes = $derived(
    (release?.notes ?? "")
      .split("\n")
      .map((l) => l.replace(/^[-*#\s]+/, "").trim())
      .filter(Boolean)
      .slice(0, 4),
  );
</script>

<GlassCard title="Atlas updates" {subtitle} icon="download" large>
  {#snippet actions()}
    <Button size="sm" variant="ghost" icon="refresh" busy={s?.phase === "checking"} disabled={appUpdate.busy} onclick={() => void appUpdate.check()}>Check now</Button>
  {/snippet}
  {#if !s}
    <p class="hint">Reading…</p>
  {:else if s.phase === "downloading"}
    <ProgressBar value={s.progress} label="Downloading {release?.version ?? 'the update'}" />
    <p class="hint">Downloading {release?.installer ? bytes(release.installer.size) : ""}; it's checked before anything is installed.</p>
  {:else if s.phase === "installing"}
    <p class="flex items-center gap-2 text-[13px] text-fg"><Icon name="loader-2" size={15} class="spin" />Installing {release?.version}…</p>
    {#if s.kind === "rpm" || s.kind === "deb"}<p class="hint">Your system asks for your password to install it.</p>{/if}
  {:else if s.phase === "restarting"}
    <p class="flex items-center gap-2 text-[13px] text-fg"><Icon name="loader-2" size={15} class="spin" />Installed. Atlas is restarting…</p>
  {:else if release}
    <div class="flex flex-col gap-2">
      <p class="flex items-center gap-2 text-[13.5px] text-fg">
        <Icon name="arrow-up" size={15} /><span class="font-medium">Atlas {release.version} is out</span>
        {#if release.prerelease}<Pill tone="warning" label="Pre-release" />{/if}
        {#if release.published_at}<span class="text-[12px] text-fg-faint">{timeAgo(Date.parse(release.published_at))}</span>{/if}
      </p>
      {#if notes.length}
        <ul class="notes">
          {#each notes as line, i (i)}<li>{line}</li>{/each}
        </ul>
      {/if}
      <div class="flex flex-wrap items-center gap-2">
        {#if replaces}
          <Button size="sm" variant="primary" icon="download" onclick={() => void appUpdate.install()}>Install and restart</Button>
        {/if}
        <Button size="sm" variant="ghost" icon="external-link" onclick={() => void openExternal(release.url)}>Release notes</Button>
      </div>
      {#if !replaces}
        <p class="hint">
          {s.kind === "source" || !s.kind
            ? "This Atlas wasn't installed from a package, so it can't replace itself; get the new one from the release page."
            : "The release has no installer for this computer; see the release page."}
        </p>
      {:else if !s.signing}
        <p class="hint">Checked against the SHA-256 the release publishes. This build has no release signing key yet.</p>
      {/if}
    </div>
  {:else}
    <p class="flex items-center gap-2 text-[13px] text-fg-muted">
      <Icon name="check" size={15} />{s.last_checked_ms ? `Up to date · checked ${timeAgo(s.last_checked_ms)}` : "Not checked yet"}
    </p>
  {/if}
  {#if s?.error && s.phase === "idle"}
    <p class="mt-2 flex items-start gap-2 text-[12.5px] text-err-fg" role="alert"><Icon name="alert-circle" size={15} class="mt-0.5" />{s.error}</p>
  {/if}
</GlassCard>

<style>
  .notes {
    display: flex;
    flex-direction: column;
    gap: 2px;
    padding-left: 22px;
    font-size: 12px;
    color: var(--fg-muted);
    list-style: disc;
  }
</style>
