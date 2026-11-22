// The app's state: the latest value of every topic the core publishes. A page reads a topic
// with `topic(name)`; it is `undefined` until the core has said it once.

import { call, follow, type Snapshot, type Update } from "../api/client";
import type { Inventory, Job, Scan, Settings } from "../api/types";

export const app = $state({
  /** The first snapshot has arrived. Until then pages show what they show with no data. */
  ready: false,
  version: "",
  mode: "real" as "real" | "sim",
  /** The core could not be reached at all (a browser with no bridge). */
  unreachable: "" as string,
  /** Milliseconds since the epoch, moved once a second: what "now" and "5 s ago" are read against. */
  now: Date.now(),
});

const topics = $state<Record<string, unknown>>({});

export function topic<T>(name: string): T | undefined {
  return topics[name] as T | undefined;
}

const EMPTY_INVENTORY: Inventory = { devices: [], robots: [], activity: [] };

export const inventory = () => topic<Inventory>("inventory") ?? EMPTY_INVENTORY;
export const scan = () => topic<Scan>("scan");
export const jobs = () => topic<Job[]>("jobs") ?? [];
export const settings = () => topic<Settings>("settings");

function applyTheme() {
  const theme = settings()?.theme ?? "system";
  if (theme === "system") delete document.documentElement.dataset.theme;
  else document.documentElement.dataset.theme = theme;
}

function onSnapshot(snapshot: Snapshot) {
  app.version = snapshot.version;
  app.mode = snapshot.mode;
  for (const name of Object.keys(topics)) if (!(name in snapshot.topics)) delete topics[name];
  Object.assign(topics, snapshot.topics);
  app.ready = true;
  app.unreachable = "";
  applyTheme();
}

function onUpdate(update: Update) {
  if (update.data === null) delete topics[update.topic];
  else topics[update.topic] = update.data;
  if (update.topic === "settings") applyTheme();
}

/** Says a UI error to the app's log. Never throws. */
export function logUiError(place: string, error: unknown) {
  const message = error instanceof Error ? `${error.message}${error.stack ? ` | ${error.stack.split("\n").slice(0, 4).join(" < ")}` : ""}` : String(error);
  call("log.ui_error", { place, message }).catch(() => {});
}

/** A breadcrumb: what the person did just before. Never throws. */
export function breadcrumb(message: string) {
  call("log.breadcrumb", { message }).catch(() => {});
}

export async function start() {
  setInterval(() => (app.now = Date.now()), 1000);
  window.addEventListener("error", (event) => logUiError("window", event.error ?? event.message));
  window.addEventListener("unhandledrejection", (event) => logUiError("promise", event.reason));
  // Failure rule: keep trying, at most five seconds apart.
  let wait = 250;
  for (;;) {
    try {
      await follow(onSnapshot, onUpdate);
      return;
    } catch (error) {
      app.unreachable = error instanceof Error ? error.message : String(error);
      await new Promise((done) => setTimeout(done, wait));
      wait = Math.min(wait * 2, 5000);
    }
  }
}
