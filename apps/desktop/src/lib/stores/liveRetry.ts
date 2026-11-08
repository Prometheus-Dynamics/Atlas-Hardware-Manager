// The rules of the live-stream retries (live.svelte.ts), kept apart from the
// store so they can be tested without the app: how long a board that refused
// its stream waits before the next try, and why a device's readings are
// polled when its board refused this computer.

/** How long before a board without a stream is asked again, after its first refusal. */
export const STREAM_RETRY_MS = 2_000;
/**
 * The longest wait between tries (failure handling: never minutes). A board
 * that comes back is picked up within seconds.
 */
export const STREAM_RETRY_MAX_MS = 10_000;

/**
 * The wait after a refusal. The first is STREAM_RETRY_MS; each refusal after
 * doubles the wait before it, up to STREAM_RETRY_MAX_MS.
 */
export function nextStreamWait(previous: number | undefined): number {
  return previous === undefined ? STREAM_RETRY_MS : Math.min(previous * 2, STREAM_RETRY_MAX_MS);
}

/** Why a device's readings are polled because of its board's Orion. */
export type Pairing = "other-key" | "unenrolled";

/**
 * The pairing state of a device: null while its stream runs. The board
 * refused this computer's key (`refused`), or the stream's reason says the
 * operator isn't enrolled there.
 */
export function pairingOf(streamed: boolean, refused: boolean, reason: string | undefined): Pairing | null {
  if (streamed) return null;
  if (refused) return "other-key";
  if (reason && /isn't enrolled|not enrolled/.test(reason)) return "unenrolled";
  return null;
}
