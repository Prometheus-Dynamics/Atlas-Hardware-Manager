// Subscriptions to the events the shell forwards from atlas-core.

import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type { AtlasEvent, DownloadEvent } from "./types";

/** Every core state change. The UI is a projection of this stream. */
export function onAtlasEvent(handler: (event: AtlasEvent) => void): Promise<UnlistenFn> {
  return listen<AtlasEvent>("atlas://event", (message) => handler(message.payload));
}

/** Fired when the UI fell behind the event stream; reload lists from commands. */
export function onResync(handler: () => void): Promise<UnlistenFn> {
  return listen("atlas://resync", () => handler());
}

export function onDownloadProgress(handler: (event: DownloadEvent) => void): Promise<UnlistenFn> {
  return listen<DownloadEvent>("atlas://download", (message) => handler(message.payload));
}
