// In-memory stand-in for the Tauri event channels used by the browser mock.

import type { AtlasEvent, DownloadEvent } from "../types";

type Unlisten = () => void;

function channel<T>() {
  const handlers = new Set<(payload: T) => void>();
  return {
    listen(handler: (payload: T) => void): Promise<Unlisten> {
      handlers.add(handler);
      return Promise.resolve(() => handlers.delete(handler));
    },
    emit(payload: T) {
      // Deliver asynchronously, like Tauri does.
      queueMicrotask(() => handlers.forEach((handler) => handler(payload)));
    },
  };
}

export const atlasChannel = channel<AtlasEvent>();
export const resyncChannel = channel<void>();
export const downloadChannel = channel<DownloadEvent>();

export const emit = (event: AtlasEvent) => atlasChannel.emit(event);

export const sleep = (ms: number) => new Promise<void>((resolve) => setTimeout(resolve, ms));

/** Simulated command latency so pending states are visible. */
export const latency = () => sleep(120 + Math.random() * 180);
