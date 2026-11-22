// A fresh app for each test: the UI bridge started on its own port with its own data directory,
// so every test sees the simulated devices from the beginning and no test sees another's state.
import { test as base, expect, type Page } from "@playwright/test";
import { spawn, type ChildProcess } from "node:child_process";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";

const BRIDGE = process.env.ATLAS_UI_BRIDGE ?? "";
const DIST = resolve(import.meta.dirname, "../dist");

export type Bridge = {
  url: string;
  dataDir: string;
  /** Asks the core directly, as the test harness (not as a click). Used only to change the
   *  simulated world: unplug a board, bring it back. */
  call(name: string, args?: unknown): Promise<unknown>;
  stop(): Promise<void>;
};

export type Scene = "demo" | "empty" | "plain";

export async function startBridge(dataDir: string, scene: Scene = "demo"): Promise<Bridge> {
  if (!BRIDGE) throw new Error("ATLAS_UI_BRIDGE is not set: run scripts/ui-gate.sh");
  const child: ChildProcess = spawn(BRIDGE, ["--dist", DIST, "--port", "0", "--data", dataDir, "--scene", scene], {
    stdio: ["ignore", "pipe", "inherit"],
  });
  const url = await new Promise<string>((done, fail) => {
    let text = "";
    child.stdout!.on("data", (chunk) => {
      text += chunk;
      const found = /listening on (http:\/\/127\.0\.0\.1:\d+)/.exec(text);
      if (found) done(found[1]!);
    });
    child.once("exit", (code) => fail(new Error(`the bridge exited (${code}) before it listened`)));
  });
  return {
    url,
    dataDir,
    async call(name, args) {
      const reply = await (
        await fetch(`${url}/call`, {
          method: "POST",
          headers: { "Content-Type": "application/json", Origin: url },
          body: JSON.stringify({ name, args: args ?? null }),
        })
      ).json();
      if (reply.error !== undefined) throw new Error(reply.error);
      return reply.ok;
    },
    stop: () =>
      new Promise<void>((done) => {
        if (child.exitCode !== null) return done();
        child.once("exit", () => done());
        child.kill();
      }),
  };
}

export const test = base.extend<{ scene: Scene; bridge: Bridge; app: Page }>({
  // The simulated world a test runs in: `test.use({ scene: "empty" })` for nothing plugged in.
  scene: ["demo", { option: true }],
  bridge: async ({ scene }, use) => {
    const dataDir = mkdtempSync(join(tmpdir(), "atlas-e2e-"));
    const bridge = await startBridge(dataDir, scene);
    await use(bridge);
    await bridge.stop();
    rmSync(dataDir, { recursive: true, force: true });
  },
  // The page, opened on the app, with every console error and page error failing the test.
  app: async ({ page, bridge }, use) => {
    const problems: string[] = [];
    page.on("console", (message) => {
      if (message.type() === "error") problems.push(`console: ${message.text()}`);
    });
    page.on("pageerror", (error) => problems.push(`page error: ${error.message}`));
    await page.goto(bridge.url);
    await use(page);
    expect(problems, "the page logged errors").toEqual([]);
  },
});

export { expect };

/** Fails when the page scrolls or any element's box leaves the viewport (design language,
 *  "No page scroll, ever"). */
export async function expectNoOverflow(page: Page) {
  const report = await page.evaluate(() => {
    const root = document.documentElement;
    const out: string[] = [];
    if (root.scrollHeight > root.clientHeight || root.scrollWidth > root.clientWidth) {
      out.push(`the document scrolls: ${root.scrollWidth}x${root.scrollHeight} in ${root.clientWidth}x${root.clientHeight}`);
    }
    for (const element of document.querySelectorAll<HTMLElement>(".app *")) {
      const box = element.getBoundingClientRect();
      if (box.width === 0 || box.height === 0) continue;
      if (box.right > root.clientWidth + 0.5 || box.bottom > root.clientHeight + 0.5) {
        // A box cut off by a parent that hides overflow is not seen past the viewport.
        let clipped = false;
        for (let parent = element.parentElement; parent; parent = parent.parentElement) {
          const style = getComputedStyle(parent);
          const edge = parent.getBoundingClientRect();
          if (style.overflow !== "visible" && edge.right <= root.clientWidth + 0.5 && edge.bottom <= root.clientHeight + 0.5) {
            clipped = true;
            break;
          }
        }
        if (!clipped) out.push(`${element.tagName.toLowerCase()}.${element.className} leaves the viewport`);
      }
    }
    return out;
  });
  expect(report, "overflow").toEqual([]);
}

/** The three window sizes the design language is checked at. */
export const SIZES = [
  { width: 1024, height: 680 },
  { width: 1320, height: 860 },
  { width: 1830, height: 1030 },
] as const;

/** The demo world's records by the name each is shown under, and the id a call names it by. */
export const DEMO = {
  controller: { name: "sc-practice", id: "systemcore:9999" },
  radio: { name: "field-radio", id: "frc-radio:9999" },
  running: { name: "bench-a", id: "simulated:0123456789abcdef" },
  bootRom: { name: "bench-b", id: "rockchip-maskrom:usb-1-3" },
  recovery: { name: "bench-c", id: "simulated:SIM-REC-1" },
  offline: { name: "old-bench", id: "simulated:SIM-OLD-1" },
  disk: { name: "SD card 64 GB", id: "removable-disk:SIM-CARD-1" },
} as const;

/** Waits until the demo world has settled in the tree: every record found, named, and the old
 *  board gone offline. */
export async function demoSettled(page: Page) {
  const side = page.locator("aside.side");
  await expect(side.locator("a.nav .dot")).toHaveCount(7);
  await expect(side.locator(".sec")).toHaveText(["Devices", "Offline", "Disks", "Tools"]);
  for (const record of Object.values(DEMO)) await expect(side.getByText(record.name, { exact: true })).toBeVisible();
}
