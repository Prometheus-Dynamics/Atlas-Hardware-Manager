// App info, settings, and host health.

import { api, errorText, type AppInfo, type AppSettings, type HealthCheck } from "#lib/api/client.ts";
import { toasts } from "./toasts.svelte";

const ORDER = { error: 0, warning: 1, ok: 2 } as const;

class SystemStore {
  info = $state<AppInfo | null>(null);
  settings = $state<AppSettings | null>(null);
  health = $state<HealthCheck[]>([]);
  checking = $state(false);
  restartNeeded = $state(false);

  healthSorted = $derived([...this.health].sort((a, b) => ORDER[a.status] - ORDER[b.status]));
  healthProblems = $derived(this.health.filter((h) => h.status !== "ok").length);

  async load() {
    try {
      const [info, settings] = await Promise.all([api.appInfo(), api.getSettings()]);
      this.info = info;
      this.settings = settings;
    } catch (error) {
      toasts.error(`Could not load settings: ${errorText(error)}`);
    }
    await this.checkHealth();
  }

  async checkHealth() {
    this.checking = true;
    try {
      this.health = await api.healthChecks();
    } catch (error) {
      toasts.error(`Health checks failed: ${errorText(error)}`);
    } finally {
      this.checking = false;
    }
  }
}

export const system = new SystemStore();
