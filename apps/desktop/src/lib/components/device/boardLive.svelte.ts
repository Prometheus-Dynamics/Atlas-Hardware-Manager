// One live stream of a board's hardware for its Overview: the IMU, the
// board's fusion, the status ring and the power monitor, shared by the
// panels that draw them (Orion's board.hardware, else the board's own
// stream). Only while the Overview is shown: the board reads its devices
// only while someone watches.
import { api, keyString, type DeviceRecord, type HardwareFrame, type HardwareSnapshot } from "#lib/api/client.ts";
import { SvelteMap } from "svelte/reactivity";
import { LiveSeries, periodFor } from "./live.ts";

// "power-monitor" is Lemnos's name for the INA238; "power" the simulated board's.
const CLASSES = ["imu", "orientation", "light", "power-monitor", "power"];

export class BoardLive {
  series = new SvelteMap<string, LiveSeries>();
  /** Bumped as samples arrive (at most ~10 times a second), so readouts follow. */
  tick = $state(0);
  state = $state<"none" | "connecting" | "live" | "stopped">("none");
  private stop: (() => void) | null = null;
  private key = "";
  private dirty = false;
  private timer: ReturnType<typeof setInterval> | null = null;

  /** The device of `cls` the snapshot lists, if any. */
  static find(hardware: HardwareSnapshot | null | undefined, cls: string) {
    return hardware?.devices.find((d) => d.class === cls && d.status !== "missing") ?? null;
  }

  follow(record: DeviceRecord, hardware: HardwareSnapshot | null | undefined) {
    const wanted = CLASSES.flatMap((cls) => {
      const device = BoardLive.find(hardware, cls);
      return device ? [[device.id, periodFor(cls)] as [string, number]] : [];
    });
    const key = `${keyString(record.key)} ${wanted.map((w) => w.join(":")).join(",")}`;
    if (key === this.key) return;
    this.close();
    this.key = key;
    if (record.presence !== "online" || wanted.length === 0) return;
    let gone = false;
    this.state = "connecting";
    this.timer = setInterval(() => {
      if (this.dirty) {
        this.dirty = false;
        this.tick += 1;
      }
    }, 100);
    const onFrame = (frame: HardwareFrame) => {
      if (gone) return;
      if (frame.type === "devices") {
        for (const device of frame.devices) {
          if (device.missing || device.refused) continue;
          const had = this.series.get(device.id);
          if (!had || had.channels.map((c) => c.name).join() !== device.channels.map((c) => c.name).join()) {
            this.series.set(device.id, new LiveSeries(device));
          }
        }
        this.state = "live";
      } else if (frame.type === "samples") {
        const series = this.series.get(frame.device);
        if (series) for (const [t, values] of frame.samples) series.push(t, values);
        this.dirty = true;
      } else {
        this.state = "stopped";
      }
    };
    api
      .streamHardware(record.key, wanted, onFrame)
      .then((stopper) => {
        if (gone) stopper?.();
        else if (stopper) this.stop = () => ((gone = true), stopper());
        else this.state = "none";
      })
      .catch(() => (this.state = "stopped"));
    this.stop ??= () => (gone = true);
  }

  close() {
    this.stop?.();
    this.stop = null;
    if (this.timer) clearInterval(this.timer);
    this.timer = null;
    this.key = "";
    this.state = "none";
  }

  /** The newest value of `channel` on `device`, or null. */
  latest(device: string | null | undefined, channel: string): number | null {
    void this.tick;
    const series = device ? this.series.get(device) : undefined;
    if (!series?.count) return null;
    const c = series.channels.findIndex((ch) => ch.name === channel);
    if (c < 0) return null;
    const v = series.v[c][series.index(series.count - 1)];
    return Number.isFinite(v) ? v : null;
  }
}
