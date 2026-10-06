// Fleet history: loaded once, then extended by `activity` events.

import { api, type ActivityEntry } from "#lib/api/client.ts";

const KEEP = 300;

class ActivityStore {
  /** Newest first. */
  entries = $state<ActivityEntry[]>([]);
  loaded = $state(false);

  async load() {
    try {
      this.entries = await api.listActivity(KEEP);
      this.loaded = true;
    } catch {
      // History is a nicety; the rest of the app works without it.
      this.loaded = true;
    }
  }

  push(entry: ActivityEntry) {
    this.entries = [entry, ...this.entries].slice(0, KEEP);
  }
}

export const activity = new ActivityStore();
