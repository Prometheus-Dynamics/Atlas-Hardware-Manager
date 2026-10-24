// Fleet history: loaded once, then extended by `activity` events.

import { api, type ActivityEntry } from "#lib/api/client.ts";

const KEEP = 300;

/** The same entry (entries have no id): when, what, which device, the text. */
function same(a: ActivityEntry, b: ActivityEntry): boolean {
  return (
    a.at_ms === b.at_ms &&
    a.kind === b.kind &&
    a.message === b.message &&
    JSON.stringify(a.device) === JSON.stringify(b.device)
  );
}

class ActivityStore {
  /** Newest first. */
  entries = $state<ActivityEntry[]>([]);
  loaded = $state(false);

  /** The load in flight: an older one finishing late is dropped. */
  private loading = 0;

  async load() {
    const generation = ++this.loading;
    try {
      const fresh = await api.listActivity(KEEP);
      if (generation !== this.loading) return;
      // Events that arrived while the list loaded and aren't in it stay.
      const newest = fresh[0]?.at_ms ?? 0;
      const later = this.entries.filter((e) => e.at_ms >= newest && !fresh.some((f) => same(e, f)));
      this.entries = [...later, ...fresh].slice(0, KEEP);
      this.loaded = true;
    } catch {
      // History is a nicety; the rest of the app works without it.
      this.loaded = true;
    }
  }

  /** Adds an event's entry, unless the list has it already: an event that
   * a reload's snapshot included can still be delivered after it. */
  push(entry: ActivityEntry) {
    if (this.entries.slice(0, 50).some((e) => same(e, entry))) return;
    this.entries = [entry, ...this.entries].slice(0, KEEP);
  }
}

export const activity = new ActivityStore();
