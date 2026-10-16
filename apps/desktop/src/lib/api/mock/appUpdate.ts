// Atlas's own update for the browser build: a newer release to find,
// download and "install".
import type { AppUpdateStatus } from "../types";
import { appUpdateChannel, sleep } from "./bus";
import { settings } from "./data";

const status: AppUpdateStatus = {
  current: "2026.0.0",
  release_build: true,
  kind: "rpm",
  phase: "idle",
  progress: null,
  last_checked_ms: null,
  available: null,
  signing: true,
  error: null,
};

function changed(change: Partial<AppUpdateStatus>) {
  Object.assign(status, change);
  appUpdateChannel.emit(structuredClone(status));
}

export const appUpdateStatus = () => structuredClone(status);

export async function checkAppUpdate() {
  changed({ phase: "checking", error: null });
  await sleep(700);
  const pre = settings.app_updates.prereleases;
  changed({
    phase: "idle",
    last_checked_ms: Date.now(),
    available: {
      version: pre ? "2026.2.0-rc.1" : "2026.1.0",
      tag: pre ? "v2026.2.0-rc.1" : "v2026.1.0",
      prerelease: pre,
      published_at: new Date(Date.now() - 2 * 86_400_000).toISOString(),
      url: "https://github.com/Prometheus-Dynamics/Atlas-Hardware-Manager/releases",
      notes: "- Slots on Raze boards: install, boot once, set default\n- Atlas updates itself from GitHub",
      installer: { name: "Atlas.Hardware.Manager-2026.1.0-1.x86_64.rpm", url: "", size: 98_000_000, manifest_url: "", digest: null },
    },
  });
  return appUpdateStatus();
}

export async function installAppUpdate() {
  if (!status.available) throw "there is no newer Atlas to install";
  changed({ phase: "downloading", progress: 0, error: null });
  for (let i = 1; i <= 20; i++) {
    await sleep(120);
    changed({ progress: i / 20 });
  }
  changed({ phase: "installing", progress: null });
  await sleep(1200);
  changed({ phase: "restarting" });
  await sleep(1500);
  changed({ phase: "idle", current: status.available.version, available: null });
}

// Like the app: look a little after start when the setting is on.
if (typeof window !== "undefined") {
  setTimeout(() => {
    if (settings.app_updates.auto_check && status.phase === "idle") void checkAppUpdate();
  }, 2500);
}
