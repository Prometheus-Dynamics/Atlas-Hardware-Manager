import { expect, test } from "bun:test";
import { nextStreamWait, pairingOf, STREAM_RETRY_MAX_MS, STREAM_RETRY_MS } from "./liveRetry";

test("a refused stream is retried after 2 s, then twice as long each refusal", () => {
  expect(STREAM_RETRY_MS).toBe(2_000);
  const waits: number[] = [];
  let wait: number | undefined;
  for (let i = 0; i < 4; i++) {
    wait = nextStreamWait(wait);
    waits.push(wait);
  }
  expect(waits).toEqual([2_000, 4_000, 8_000, 10_000]);
});

test("the retry wait never exceeds 10 s, however many refusals", () => {
  expect(STREAM_RETRY_MAX_MS).toBe(10_000);
  let wait: number | undefined;
  for (let i = 0; i < 100; i++) wait = nextStreamWait(wait);
  expect(wait).toBe(10_000);
  expect(nextStreamWait(10_000)).toBe(10_000);
});

test("a streamed device is never in a pairing state", () => {
  expect(pairingOf(true, true, "the board refuses this computer's Orion key")).toBeNull();
  expect(pairingOf(true, false, "not enrolled")).toBeNull();
});

test("a board that refused this computer's key is other-key, whatever the reason text", () => {
  expect(pairingOf(false, true, "the board's stream refused: whatever")).toBe("other-key");
  expect(pairingOf(false, true, undefined)).toBe("other-key");
});

test("a board that doesn't know the operator is unenrolled", () => {
  expect(pairingOf(false, false, "the board's stream refused: operator x is not enrolled")).toBe("unenrolled");
  expect(pairingOf(false, false, "the board's stream refused: this PC isn't enrolled")).toBe("unenrolled");
});

test("other failures and no reason are not a pairing state", () => {
  expect(pairingOf(false, false, "no readings from the board's stream in 3 s")).toBeNull();
  expect(pairingOf(false, false, undefined)).toBeNull();
});
