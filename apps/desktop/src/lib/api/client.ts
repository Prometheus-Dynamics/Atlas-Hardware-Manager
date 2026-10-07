// The one import point for components: the real Tauri commands and events
// under Tauri, the in-memory simulated backend in a plain browser.

import { api as tauriApi, errorText } from "./commands";
import * as tauriEvents from "./events";
import { mockApi, mockEvents } from "./mock";

export * from "./types";
export { errorText };

/** True inside the Tauri shell; false in a plain browser preview. */
export const isTauri = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

export const api: typeof tauriApi = isTauri ? tauriApi : mockApi;
export const onAtlasEvent = isTauri ? tauriEvents.onAtlasEvent : mockEvents.onAtlasEvent;
export const onResync = isTauri ? tauriEvents.onResync : mockEvents.onResync;
export const onDownloadProgress = isTauri ? tauriEvents.onDownloadProgress : mockEvents.onDownloadProgress;

const RELEASE_FILTERS = [
  { name: "Images and firmware", extensions: ["img", "xz", "zst", "gz", "bin", "hex", "zip"] },
  { name: "Any file", extensions: ["*"] },
];

const BROWSER_PICKS = [
  "/home/atlas/Downloads/helios-raze-2026.3.1-rc1.img.xz",
  "/home/atlas/builds/raze-custom.img",
];
let picks = 0;

/** Asks for a release file; null when the user cancels. */
export async function pickReleaseFile(): Promise<string | null> {
  if (!isTauri) {
    // Browser preview: no native picker, so pretend the user chose a build.
    await new Promise((resolve) => setTimeout(resolve, 250));
    return BROWSER_PICKS[picks++ % BROWSER_PICKS.length];
  }
  const { open } = await import("@tauri-apps/plugin-dialog");
  const picked = await open({ multiple: false, directory: false, filters: RELEASE_FILTERS });
  return typeof picked === "string" ? picked : null;
}

/** Asks where to save a file; null when the user cancels. */
export async function pickSavePath(defaultName: string): Promise<string | null> {
  if (!isTauri) {
    await new Promise((resolve) => setTimeout(resolve, 200));
    return `/home/atlas/Downloads/${defaultName}`;
  }
  const { save } = await import("@tauri-apps/plugin-dialog");
  return save({ defaultPath: defaultName, filters: [{ name: "Text", extensions: ["txt"] }] });
}

/** Opens a URL in the system browser. */
export async function openExternal(url: string): Promise<void> {
  if (!isTauri) {
    window.open(url, "_blank", "noopener");
    return;
  }
  const { openUrl } = await import("@tauri-apps/plugin-opener");
  await openUrl(url);
}
