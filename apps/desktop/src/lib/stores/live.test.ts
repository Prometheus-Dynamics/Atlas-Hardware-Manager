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

test("streamed rows are held and drawn together; a device no view watches drops them", () => {
  const store = live as unknown as {
    queue(id: string, frame: LiveFrame): void;
    applyQueued(): void;
    watchers: Map<string, number[]>;
  };
  store.watchers.set("q1", [1000]);
  store.watchers.set("q2", [1000]);
  const row = (t: number) => ({ kind: "rows", backfill: false, rows: [{ t, values: { cpu: t } }], latest: [reading("cpu", t)] }) as LiveFrame;
  store.queue("q1", row(1));
  store.queue("q2", row(2));
  store.queue("gone", row(3));
  expect(live.metrics.has("q1")).toBe(false);
  store.applyQueued();
  expect(live.metric("q1", "cpu")?.value).toBe(1);
  expect(live.metric("q2", "cpu")?.value).toBe(2);
  expect(live.metrics.has("gone")).toBe(false);
});
