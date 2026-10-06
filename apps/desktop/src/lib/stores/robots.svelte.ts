// Robot profiles and their computed status.

import { api, errorText, type RobotProfile, type RobotStatus } from "#lib/api/client.ts";
import { toasts } from "./toasts.svelte";

class RobotStore {
  profiles = $state<RobotProfile[]>([]);
  statuses = $state<RobotStatus[]>([]);
  loaded = $state(false);

  names = $derived(this.profiles.map((p) => p.name).sort());

  status(name: string): RobotStatus | undefined {
    return this.statuses.find((s) => s.name === name);
  }

  profile(name: string): RobotProfile | undefined {
    return this.profiles.find((p) => p.name === name);
  }

  async load() {
    try {
      const [profiles, statuses] = await Promise.all([api.listRobots(), api.robotStatuses()]);
      this.profiles = profiles;
      this.statuses = statuses;
      this.loaded = true;
    } catch (error) {
      toasts.error(`Could not load robots: ${errorText(error)}`);
    }
  }

  private statusTimer: ReturnType<typeof setTimeout> | null = null;

  /**
   * Statuses depend on device versions and presence, which change without a
   * robots-changed event, so they are re-read (debounced) after device events.
   */
  refreshStatusesSoon() {
    if (this.statusTimer) return;
    this.statusTimer = setTimeout(async () => {
      this.statusTimer = null;
      try {
        this.statuses = await api.robotStatuses();
      } catch {
        // Keep the last statuses; the next event retries.
      }
    }, 600);
  }
}

export const robots = new RobotStore();
