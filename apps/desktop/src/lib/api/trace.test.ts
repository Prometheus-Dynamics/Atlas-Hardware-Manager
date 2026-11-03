import { describe, expect, mock, test } from "bun:test";

const logged: { kind: string; message: string }[] = [];
mock.module("@tauri-apps/api/core", () => ({
  Channel: class {},
  invoke: async (cmd: string, args: Record<string, string>) => {
    if (cmd === "report_ui_error") return void logged.push({ kind: args.kind, message: args.message });
    throw `couldn't reach ${cmd}`;
  },
}));
(globalThis as { location?: unknown }).location ??= { pathname: "/device" };
const { crumb, errorId, invoke } = await import("./trace.ts");

describe("a failed command", () => {
  test("is logged once with an id, the user's last steps, and the id its toast shows", async () => {
    crumb('click "Restart port"');
    await expect(invoke("restart_port")).rejects.toBe("couldn't reach restart_port");
    await expect(invoke("restart_port")).rejects.toBe("couldn't reach restart_port");
    await Promise.resolve();
    expect(logged).toHaveLength(1);
    const [line] = logged;
    expect(line.kind).toBe("UI-CALL");
    expect(line.message).toMatch(/^\[[0-9a-f]{6}\] restart_port failed after [\d.]+ s: couldn't reach restart_port/);
    expect(line.message).toContain('click "Restart port"');
    const id = line.message.slice(1, 7);
    expect(errorId("Couldn't restart: couldn't reach restart_port")).toBe(id);
    expect(errorId("Something else")).toBeNull();
  });
});
