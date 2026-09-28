// Release catalog, remote sources, and download progress.

import { SvelteMap } from "svelte/reactivity";
import { api, errorText, type DownloadEvent, type ReleaseEntry, type RemoteSource } from "$lib/api/client";
import { compareVersionsDesc } from "$lib/format";
import { toasts } from "./toasts.svelte";

class ReleaseStore {
  entries = $state<ReleaseEntry[]>([]);
  sources = $state<RemoteSource[]>([]);
  loaded = $state(false);
  warnings = $state<string[]>([]);
  downloads = new SvelteMap<string, DownloadEvent>();

  families = $derived([...new Set(this.entries.map((e) => e.family))].sort());

  /** Entries for one family, newest first. */
  forFamily(family: string): ReleaseEntry[] {
    return this.entries.filter((e) => e.family === family).sort((a, b) => compareVersionsDesc(a.version, b.version));
  }

  async load() {
    try {
      const [entries, sources] = await Promise.all([api.listReleases(), api.listReleaseSources()]);
      this.entries = entries;
      this.sources = sources;
      this.loaded = true;
    } catch (error) {
      toasts.error(`Could not load releases: ${errorText(error)}`);
    }
  }

  onDownload(event: DownloadEvent) {
    this.downloads.set(event.id, event);
  }

  async download(id: string) {
    if (this.downloads.has(id)) return;
    this.downloads.set(id, { id, downloaded: 0, total: null });
    try {
      const entry = await api.downloadRelease(id);
      const index = this.entries.findIndex((e) => e.id === id);
      if (index >= 0) this.entries[index] = entry;
      toasts.success(`Downloaded ${entry.artifact_name}.`);
    } catch (error) {
      toasts.error(`Download failed: ${errorText(error)}`);
    } finally {
      this.downloads.delete(id);
    }
  }
}

export const releases = new ReleaseStore();
