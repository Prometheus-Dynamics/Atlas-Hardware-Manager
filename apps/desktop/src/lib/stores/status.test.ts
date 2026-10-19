// The status store's reads of an idle board (bun test): a view that leaves
// and comes back within BUSY_MS of the last read must still get its next
// read; before 5185c12 it got none until the board pushed something.
import { afterEach, beforeEach, expect, jest, mock, test } from "bun:test";
import type { DeviceRecord, DeviceStatus } from "#lib/api/types.ts";

const reads: string[] = [];
mock.module("#lib/api/client.ts", () => ({
  api: {
    deviceStatus: async (_key: unknown, caller?: string) => {
      reads.push(caller ?? "?");
      return { update: null, pushed: true } as unknown as DeviceStatus;
    },
    deviceHistory: async () => [],
  },
  errorText: (e: unknown) => String(e),
  keyString: (key: { family: string; serial: string }) => `${key.family}:${key.serial}`,
}));
const { deviceStatus } = await import("./status.svelte.ts");

const record = {
  key: { family: "raze", serial: "0708c973b00a37bc" },
  presence: "online",
  capabilities: ["status"],
} as unknown as DeviceRecord;

/** Runs the timers due in the next `ms` and lets the reads they start answer. */
async function pass(ms: number) {
  jest.advanceTimersByTime(ms);
  for (let i = 0; i < 5; i++) await Promise.resolve();
}

beforeEach(() => {
  jest.useFakeTimers();
  reads.length = 0;
});
afterEach(() => jest.useRealTimers());

test("a view back within 3 s of the last read still gets the next read", async () => {
  const stop = deviceStatus.watch(record);
  await pass(0);
  expect(reads).toEqual(["status-store watch"]);

  // Every view leaves; a second later the store drops the device and its timer.
  stop();
  await pass(1100);
  // Back 1.2 s after the read: too soon to read again at once…
  const again = deviceStatus.watch(record);
  await pass(0);
  expect(reads).toHaveLength(1);
  // …but read at 3 s after the last one, then every minute (a board that pushes).
  await pass(1900);
  expect(reads).toEqual(["status-store watch", "status-store watch, after a recent read"]);
  await pass(60_000);
  expect(reads.at(-1)).toBe("status-store timer 60 s");
  again();
  await pass(1100);
});
