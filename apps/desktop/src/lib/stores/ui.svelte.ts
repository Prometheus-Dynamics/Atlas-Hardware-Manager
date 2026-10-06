// View state: side panel, inventory selection and filters, help.

import { SvelteSet } from "svelte/reactivity";
import { keyString, type DeviceRecord, type JobId, type UpdateRequestInput } from "#lib/api/client.ts";
import { deviceName } from "#lib/format.ts";
import { devices } from "./devices.svelte";

export type Panel =
  | { kind: "device"; key: string; tab?: string }
  | { kind: "robot"; name: string | null }
  | { kind: "update"; request: UpdateRequestInput; title: string }
  | null;

/** Robot filter value meaning "no robot assigned". */
export const NO_ROBOT = "\u0000none";

export type InventoryView = "cards" | "list";

const VIEW_KEY = "atlas.inventory.view";

function savedView(): InventoryView {
  try {
    return localStorage.getItem(VIEW_KEY) === "list" ? "list" : "cards";
  } catch {
    return "cards";
  }
}

export function canUpdate(record: DeviceRecord): boolean {
  return (
    record.presence === "online" && (record.capabilities.includes("update") || record.capabilities.includes("recover"))
  );
}

class UiStore {
  panel = $state<Panel>(null);
  helpOpen = $state(false);

  selection = new SvelteSet<string>();
  anchor = $state<string | null>(null);
  focused = $state<string | null>(null);

  filterText = $state("");
  families = new SvelteSet<string>();
  robot = $state<string | null>(null);
  showOffline = $state(true);

  selectedJob = $state<JobId | null>(null);
  trayOpen = $state(true);

  /** Inventory layout, remembered per machine. */
  view = $state<InventoryView>(savedView());
  /** "Needs you" banners the user dismissed this session. */
  dismissed = new SvelteSet<string>();

  /** Group the inventory by robot when no robot filter is active. */
  grouped = $derived(this.robot === null && devices.all.some((d) => d.robot));

  /** Filtered inventory in display order: online first, then by name. */
  visible = $derived.by(() => {
    const text = this.filterText.trim().toLowerCase();
    return devices.all
      .filter((d) => this.showOffline || d.presence === "online")
      .filter((d) => this.families.size === 0 || this.families.has(d.key.family))
      .filter((d) => this.robot === null || (this.robot === NO_ROBOT ? !d.robot : d.robot === this.robot))
      .filter((d) => {
        if (!text) return true;
        const hay = [deviceName(d), d.identity.name, d.key.serial, d.key.family, d.identity.model, d.robot]
          .filter(Boolean)
          .join(" ")
          .toLowerCase();
        return hay.includes(text);
      })
      .sort((a, b) => {
        if (this.grouped && a.robot !== b.robot) {
          if (!a.robot) return 1;
          if (!b.robot) return -1;
          return a.robot.localeCompare(b.robot, undefined, { numeric: true });
        }
        if (a.presence !== b.presence) return a.presence === "online" ? -1 : 1;
        return deviceName(a).localeCompare(deviceName(b), undefined, { numeric: true });
      });
  });

  selectedRecords = $derived(
    [...this.selection].map((k) => devices.get(k)).filter((d): d is DeviceRecord => !!d),
  );
  updatable = $derived(this.selectedRecords.filter(canUpdate));

  filtersActive = $derived(
    this.filterText.trim() !== "" || this.families.size > 0 || this.robot !== null || !this.showOffline,
  );

  filterInput: HTMLInputElement | null = null;

  setView(view: InventoryView) {
    this.view = view;
    try {
      localStorage.setItem(VIEW_KEY, view);
    } catch {
      // Private mode or blocked storage: the choice lasts this session.
    }
  }

  clearFilters() {
    this.filterText = "";
    this.families.clear();
    this.robot = null;
    this.showOffline = true;
  }

  openDevice(key: string, tab?: string) {
    this.panel = { kind: "device", key, tab };
    this.focused = key;
  }

  openRobot(name: string | null) {
    this.panel = { kind: "robot", name };
  }

  openUpdate(request: UpdateRequestInput, title: string) {
    this.panel = { kind: "update", request, title };
  }

  /** Update flow for the updatable part of the current selection. */
  updateSelection(staged: UpdateRequestInput["staged"]) {
    const records = this.updatable;
    if (records.length === 0) return;
    const recover = records.every((r) => r.identity.mode === "recovery");
    this.openUpdate(
      { devices: records.map((r) => r.key), releases: {}, staged },
      `${recover ? "Flash" : "Update"} ${records.length} device${records.length === 1 ? "" : "s"}`,
    );
  }

  close() {
    this.panel = null;
  }

  toggle(key: string) {
    if (this.selection.has(key)) this.selection.delete(key);
    else this.selection.add(key);
    this.anchor = key;
  }

  /** Shift-click: select everything between the anchor and this row. */
  selectRange(key: string) {
    const order = this.visible.map((d) => keyString(d.key));
    const from = this.anchor ? order.indexOf(this.anchor) : -1;
    const to = order.indexOf(key);
    if (from < 0 || to < 0) return this.toggle(key);
    const [lo, hi] = from < to ? [from, to] : [to, from];
    for (const k of order.slice(lo, hi + 1)) this.selection.add(k);
  }

  selectAllVisible() {
    const keys = this.visible.map((d) => keyString(d.key));
    const all = keys.every((k) => this.selection.has(k));
    if (all) keys.forEach((k) => this.selection.delete(k));
    else keys.forEach((k) => this.selection.add(k));
  }

  moveFocus(delta: number) {
    const order = this.visible.map((d) => keyString(d.key));
    if (order.length === 0) return;
    const index = this.focused ? order.indexOf(this.focused) : -1;
    const next = Math.min(order.length - 1, Math.max(0, index < 0 ? 0 : index + delta));
    this.focused = order[next];
  }
}

export const ui = new UiStore();
