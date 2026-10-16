// Atlas's own update: whether a newer release is out, and installing it.

import { api, errorText, onAppUpdate, type AppUpdateStatus } from "#lib/api/client.ts";
import { toasts } from "./toasts.svelte";

class AppUpdateStore {
  status = $state<AppUpdateStatus | null>(null);
  busy = $derived(!!this.status && this.status.phase !== "idle");
  available = $derived(this.status?.available ?? null);

  /** Loads the status and follows its changes; returns a cleanup. */
  start(): () => void {
    const unlisten = onAppUpdate((status) => (this.status = status));
    void api.appUpdateStatus().then((status) => (this.status ??= status), () => {});
    return () => void unlisten.then((fn) => fn());
  }

  async check() {
    try {
      this.status = await api.checkAppUpdate();
      if (this.status.error) toasts.error(`Couldn't look for updates: ${this.status.error}`);
    } catch (error) {
      toasts.error(`Couldn't look for updates: ${errorText(error)}`);
    }
  }

  async install() {
    try {
      await api.installAppUpdate();
    } catch (error) {
      toasts.error(`The update didn't install: ${errorText(error)}`);
    }
  }
}

export const appUpdate = new AppUpdateStore();
