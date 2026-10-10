// The status ring's live frames, as board-stream sends a light (see its
// hardware.rs): samples of `offset`, `clockwise`, `brightness` and then
// `led.0`..`led.<n>`, each logical LED's colour as 0xWWRRGGBB, exactly as
// lemnosd wrote it. Logical LED 0 is the ring's top, physical LED `offset`;
// "clockwise" (Lemnos `cw`) means logical LEDs run with increasing physical
// (wire) indices, whatever that looks like from the front: physical =
// (offset ± logical) mod n, as lemnos-board's gravity.rs maps them.
import type { LiveSeries } from "./live.ts";

/** One frame in physical order, as 0xRRGGBB (white folded in). */
export interface RingFrame {
  colors: number[];
  /** The newest sample's time, so a repaint happens only on change. */
  at: number;
}

/** The newest frame of a light's series, or null when it has none. */
export function ringFrame(series: LiveSeries, previous: RingFrame | null): RingFrame | null {
  if (!series.count) return null;
  const at = series.index(series.count - 1);
  const t = series.t[at];
  if (previous && previous.at === t) return previous;
  const names = series.channels.map((c) => c.name);
  const offset = series.v[names.indexOf("offset")]?.[at] ?? 0;
  const clockwise = (series.v[names.indexOf("clockwise")]?.[at] ?? 1) >= 0.5;
  const leds = names.flatMap((name, i) => (name.startsWith("led.") ? [i] : []));
  const n = leds.length;
  if (!n) return null;
  const colors = new Array<number>(n).fill(0);
  leds.forEach((channel, logical) => {
    const value = series.v[channel][at];
    if (!Number.isFinite(value)) return;
    const wrgb = value >>> 0;
    const w = (wrgb >>> 24) & 0xff;
    const add = (c: number) => Math.min(255, c + w);
    const rgb = (add((wrgb >> 16) & 0xff) << 16) | (add((wrgb >> 8) & 0xff) << 8) | add(wrgb & 0xff);
    const physical = ((clockwise ? offset + logical : offset - logical) % n + n) % n;
    colors[physical] = rgb;
  });
  return { colors, at: t };
}
