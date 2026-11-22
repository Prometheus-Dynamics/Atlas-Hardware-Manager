// The one way the UI reaches the core: `call(name, args)` and a stream of updates.
//
// In the app the transport is Tauri (one async command, one event). In a browser it is the UI
// bridge (`atlas-ui-bridge`), which serves the same calls and updates over 127.0.0.1 with the
// simulated devices; the click tests use it. Nothing else in the UI knows which one is in use.

export type Update = { topic: string; data: unknown };
export type Snapshot = { version: string; mode: "real" | "sim"; topics: Record<string, unknown> };

type Transport = {
  call(name: string, args?: unknown): Promise<unknown>;
  /** Reads everything, then follows updates. `onSnapshot` is called again after a gap. */
  follow(onSnapshot: (snapshot: Snapshot) => void, onUpdate: (update: Update) => void): Promise<void>;
};

const inTauri = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

async function tauriTransport(): Promise<Transport> {
  const { invoke } = await import("@tauri-apps/api/core");
  const { listen } = await import("@tauri-apps/api/event");
  const call = (name: string, args?: unknown) => invoke("call", { name, args: args ?? null });
  return {
    call,
    async follow(onSnapshot, onUpdate) {
      // Listen first, so no update between the snapshot and the listener is lost; a topic's
      // update carries its whole value, so one that arrives twice is harmless.
      await listen<Update>("atlas", (event) => onUpdate(event.payload));
      onSnapshot((await call("snapshot")) as Snapshot);
    },
  };
}

function bridgeTransport(): Transport {
  let last = 0;
  async function post(name: string, args?: unknown): Promise<{ ok?: unknown; error?: string; last?: number }> {
    const reply = await fetch("/call", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ name, args: args ?? null }),
    });
    return reply.json();
  }
  const call = async (name: string, args?: unknown) => {
    const reply = await post(name, args);
    if (reply.error !== undefined) throw new Error(reply.error);
    return reply.ok;
  };
  return {
    call,
    async follow(onSnapshot, onUpdate) {
      const resync = async () => {
        const reply = await post("snapshot");
        if (reply.error !== undefined) throw new Error(reply.error);
        last = reply.last ?? 0;
        onSnapshot(reply.ok as Snapshot);
      };
      await resync();
      void (async () => {
        // Failure rule: never give up, wait at most five seconds between tries.
        let wait = 250;
        for (;;) {
          try {
            const reply = await (await fetch(`/updates?after=${last}`)).json();
            wait = 250;
            if (reply.resync) {
              await resync();
              continue;
            }
            last = reply.last;
            for (const update of reply.updates as Update[]) onUpdate(update);
          } catch {
            await new Promise((done) => setTimeout(done, wait));
            wait = Math.min(wait * 2, 5000);
          }
        }
      })();
    },
  };
}

let transport: Promise<Transport> | undefined;
function get(): Promise<Transport> {
  transport ??= inTauri ? tauriTransport() : Promise.resolve(bridgeTransport());
  return transport;
}

/** Asks the core for something. Rejects with the core's own words when it fails; the core has
 *  already written the failure to the app's log. */
export async function call<T = unknown>(name: string, args?: unknown): Promise<T> {
  try {
    return (await (await get()).call(name, args)) as T;
  } catch (error) {
    throw new Error(typeof error === "string" ? error : error instanceof Error ? error.message : String(error));
  }
}

export async function follow(onSnapshot: (snapshot: Snapshot) => void, onUpdate: (update: Update) => void) {
  await (await get()).follow(onSnapshot, onUpdate);
}
