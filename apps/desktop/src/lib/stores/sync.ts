// The single subscription to backend events. Lists are loaded once, then
// every event is dispatched to the store that owns that state.

import { goto } from "$app/navigation";
import { onAtlasEvent, onDownloadProgress, onResync, keyString, type AtlasEvent } from "#lib/api/client.ts";
import { activity } from "./activity.svelte";
import { devices } from "./devices.svelte";
import { jobs } from "./jobs.svelte";
import { releases } from "./releases.svelte";
import { robots } from "./robots.svelte";
import { selftests } from "./selftests.svelte";
import { deviceStatus } from "./status.svelte";
import { system } from "./system.svelte";
import { ui } from "./ui.svelte";

export async function loadAll() {
  await Promise.all([devices.load(), activity.load(), jobs.load(), robots.load(), releases.load(), system.load()]);
}

function dispatch(event: AtlasEvent) {
  switch (event.type) {
    case "scan-started":
      devices.scanStarted();
      break;
    case "scan-finished":
      devices.scanFinished(event.report);
      robots.refreshStatusesSoon();
      break;
    case "scan-warning":
      devices.scanWarnings = [...devices.scanWarnings, event.message];
      break;
    case "device-seen":
      devices.upsert(event.record, event.new);
      robots.refreshStatusesSoon();
      break;
    case "device-offline":
      devices.markOffline(event.key);
      robots.refreshStatusesSoon();
      break;
    case "device-forgotten": {
      devices.remove(event.key);
      const id = keyString(event.key);
      ui.selection.delete(id);
      if (ui.viewing === id) void goto("/devices");
      robots.refreshStatusesSoon();
      break;
    }
    case "robots-changed":
      void robots.load();
      break;
    case "activity":
      activity.push(event.entry);
      deviceStatus.onActivity(event.entry.device);
      break;
    case "device-history":
      deviceStatus.onHistory(event.key);
      break;
    case "device-status":
      deviceStatus.onPushed(event.key);
      break;
    case "self-test":
      selftests.apply(event.record);
      break;
    case "self-test-progress":
      selftests.progress(event.key, event);
      break;
    default:
      // The jobs page follows the newest job.
      if (event.type === "job-started") ui.selectedJob = event.job;
      jobs.apply(event);
      if (event.type === "job-finished") robots.refreshStatusesSoon();
  }
}

/** Starts the event subscriptions and the initial load; returns a cleanup. */
export function startSync(): () => void {
  const pending = [
    onAtlasEvent(dispatch),
    onResync(() => void loadAll()),
    onDownloadProgress((event) => releases.onDownload(event)),
  ];
  // Subscribe first so nothing that happens during the load is missed.
  void Promise.all(pending).then(loadAll, loadAll);
  return () => {
    for (const unlisten of pending) void unlisten.then((fn) => fn());
  };
}
