import { beforeEach, expect, test } from "bun:test";
import { loadLiveCache, saveLiveCache } from "./liveCache";

const store = new Map<string, string>();
(globalThis as { localStorage?: unknown }).localStorage = {
  getItem: (k: string) => store.get(k) ?? null,
  setItem: (k: string, v: string) => void store.set(k, v),
};
beforeEach(() => store.clear());

const metric = { id: "cpu", label: "CPU", value: 12, unit: "%" } as never;

test("the last readings come back, trimmed to a sparkline's worth", () => {
  const values = Array.from({ length: 200 }, (_, i) => i);
  saveLiveCache([["a", { metrics: [metric], history: { cpu: values }, times: values, at: 1000 }]]);
  const back = loadLiveCache(2000).get("a");
  expect(back?.metrics).toEqual([metric]);
  expect(back?.history.cpu.length).toBe(60);
  expect(back?.history.cpu.at(-1)).toBe(199);
  expect(back?.times.length).toBe(60);
});

test("readings older than a day, devices without readings and a broken cache are dropped", () => {
  saveLiveCache([
    ["old", { metrics: [metric], history: {}, times: [], at: 0 }],
    ["none", { metrics: [], history: {}, times: [], at: 1000 }],
  ]);
  expect([...loadLiveCache(25 * 3600_000).keys()]).toEqual([]);
  store.set("atlas.live.v1", "{not json");
  expect(loadLiveCache().size).toBe(0);
});
