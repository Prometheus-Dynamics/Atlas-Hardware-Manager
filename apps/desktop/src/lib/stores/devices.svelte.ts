// Inventory projection: loaded once, then kept current by device events.

import { SvelteMap, SvelteSet } from "svelte/reactivity";
import { api, errorText, keyString, type DeviceKey, type DeviceRecord, type DiscoveryStatus, type ScanReport } from "#lib/api/client.ts";
import { deviceName } from "#lib/format.ts";
import { toasts } from "./toasts.svelte";

const FRESH_MS = 2500;

class DeviceStore {
  map = new SvelteMap<string, DeviceRecord>();
  /** Keys that just appeared, for a brief highlight. */
  fresh = new SvelteSet<string>();
  loaded = $state(false);
  scanning = $state(false);
  lastScan = $state<ScanReport | null>(null);
  lastScanAt = $state<number | null>(null);
  scanWarnings = $state<string[]>([]);
  discovery = $state<DiscoveryStatus | null>(null);

  all = $derived([...this.map.values()]);
  online = $derived(this.all.filter((d) => d.presence === "online").length);
  offline = $derived(this.all.length - this.online);
  families = $derived([...new Set(this.all.map((d) => d.key.family))].sort());

  get(key: DeviceKey | string): DeviceRecord | undefined {
    return this.map.get(typeof key === "string" ? key : keyString(key));
  }

  nameOf = (key: DeviceKey | string): string => {
    const record = this.get(key);
    return record ? deviceName(record) : typeof key === "string" ? key : keyString(key);
  };

  async loadDiscovery() {
    try {
      this.discovery = await api.discoveryStatus();
    } catch {
      this.discovery = null;
    }
  }

  async load() {
    void this.loadDiscovery();
    try {
      const list = await api.listDevices();
      this.map.clear();
      for (const record of list) this.map.set(keyString(record.key), record);
      this.loaded = true;
    } catch (error) {
      toasts.error(`Could not load devices: ${errorText(error)}`);
    }
  }

  upsert(record: DeviceRecord, isNew: boolean) {
    const id = keyString(record.key);
    const known = this.map.has(id);
    this.map.set(id, record);
    if (isNew || (!known && this.loaded)) {
      this.fresh.add(id);
      setTimeout(() => this.fresh.delete(id), FRESH_MS);
    }
  }

  markOffline(key: DeviceKey) {
    const id = keyString(key);
    const record = this.map.get(id);
    if (record) this.map.set(id, { ...record, presence: "offline", capabilities: ["info"] });
  }

  remove(key: DeviceKey) {
    this.map.delete(keyString(key));
  }

  scanStarted() {
    this.scanning = true;
  }

  scanFinished(report: ScanReport) {
    this.scanning = false;
    this.lastScan = report;
    this.lastScanAt = Date.now();
    this.scanWarnings = report.warnings;
  }

  async scanNow() {
    if (this.scanning) return;
    this.scanning = true;
    try {
      const report = await api.scan();
      this.scanFinished(report);
    } catch (error) {
      this.scanning = false;
      toasts.error(`Scan failed: ${errorText(error)}`);
    }
  }
}

export const devices = new DeviceStore();
