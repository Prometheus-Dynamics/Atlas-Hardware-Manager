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

/** Asks for a release file; null when the user cancels. */
export async function pickReleaseFile(): Promise<string | null> {
  if (!isTauri) {
    return window.prompt("Path to a release file (browser preview)", "/home/atlas/builds/custom.img.xz");
  }
  const { open } = await import("@tauri-apps/plugin-dialog");
  const picked = await open({ multiple: false, directory: false, filters: RELEASE_FILTERS });
  return typeof picked === "string" ? picked : null;
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
