// The live store's history (bun test): a reading polled before the stream's
// backfill arrives has no past, so the backfill mustn't invent one.
import { expect, mock, test } from "bun:test";
import type { DeviceKey, LiveFrame, Metric } from "#lib/api/types.ts";

mock.module("#lib/api/client.ts", () => ({ api: {}, errorText: String, keyString: (key: DeviceKey) => JSON.stringify(key) }));
mock.module("./devices.svelte", () => ({ devices: { get: () => undefined } }));

const { live } = await import("./live.svelte.ts");

const reading = (id: string, value: number) => ({ id, label: id, value }) as Metric;
const push = (frame: LiveFrame) => (live as unknown as { frame(id: string, frame: LiveFrame): void }).frame("board", frame);

test("a backfill draws nothing for a reading it doesn't carry", () => {
  live.metrics.set("board", [reading("volts", 12.5)]);
  push({ kind: "rows", backfill: true, rows: [1, 2, 3].map((t) => ({ t, values: { cpu: t } })), latest: [] });
  expect(live.series("board", "cpu")).toEqual([1, 2, 3]);
  expect(live.series("board", "volts")).toEqual([]);

  // From the next row on it is held between polls, lined up with the newest time.
  push({ kind: "rows", backfill: false, rows: [{ t: 4, values: { cpu: 4 } }], latest: [] });
  expect(live.window("board", "volts", 0)).toEqual([{ t: 4, v: 12.5 }]);
});

test("a chart starts at the first reading, however few there are", () => {
  live.times.set("a", [50_000, 60_000]);
  live.times.set("b", [58_000]);
  expect(live.chartStart(["a", "b"], 0, 61_000)).toBe(50_000);
  expect(live.chartStart(["b"], 0, 60_000)).toBe(50_000);
  expect(live.chartStart(["a"], 55_000, 200_000)).toBe(60_000);
});
