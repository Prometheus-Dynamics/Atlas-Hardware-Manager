// A fresh app for each test: the UI bridge started on its own port with its own data directory,
// so every test sees the simulated devices from the beginning and no test sees another's state.
import { test as base, expect, type Page } from "@playwright/test";
import { spawn, type ChildProcess } from "node:child_process";
import { copyFileSync, existsSync, mkdtempSync, rmSync } from "node:fs";
import { setTimeout as sleep } from "node:timers/promises";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";

const BRIDGE = process.env.ATLAS_UI_BRIDGE ?? "";
// Set by scripts/ui-gate.sh, which starts the tests while it still builds the bridge: the file
// appears once this run's bridge is in place.
const READY = process.env.ATLAS_UI_BRIDGE_READY ?? "";
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

/** How many times shorter the bridge makes every wait between tries (`--retry-scale`): a job's
 *  backoff and the update feed's double from a tenth of the app's first wait to a tenth of its
 *  5 s cap. A test that needs the app's own waits says `test.use({ retryScale: 1 })`. */
export const RETRY_SCALE = 10;
/** The app's cap on a wait between tries (5 s), as a bridge with `scale` runs it. */
export const capMs = (scale: number = RETRY_SCALE) => 5000 / scale;

/** Starts the bridge. With `requestLog`, the bridge writes each request's begin and end there (see
 *  `--request-log`), so a request that stalls is seen from the server's side too. */
export async function startBridge(dataDir: string, scene: Scene = "demo", requestLog?: string, retryScale: number = RETRY_SCALE): Promise<Bridge> {
  if (!BRIDGE) throw new Error("ATLAS_UI_BRIDGE is not set: run scripts/ui-gate.sh");
  const extra = ["--retry-scale", String(retryScale), ...(requestLog ? ["--request-log", requestLog] : [])];
  const child: ChildProcess = spawn(BRIDGE, ["--dist", DIST, "--port", "0", "--data", dataDir, "--scene", scene, ...extra], {
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

export const test = base.extend<{ scene: Scene; retryScale: number; bridge: Bridge; app: Page }, { bridgeBuilt: void }>({
  // Once per worker: waits until the gate says this run's bridge is built. No timeout of its own:
  // a build that fails ends the gate, and the tests with it.
  bridgeBuilt: [
    async ({}, use) => {
      while (READY && !existsSync(READY)) await sleep(50);
      await use();
    },
    { scope: "worker", timeout: 0 },
  ],
  // The simulated world a test runs in: `test.use({ scene: "empty" })` for nothing plugged in.
  scene: ["demo", { option: true }],
  // How many times shorter the waits between tries are (see RETRY_SCALE).
  retryScale: [RETRY_SCALE, { option: true }],
  // A test that fails keeps, beside its output (and its trace), the bridge's request log and the
  // app's own log; a test that passes keeps neither.
  bridge: async ({ scene, retryScale, bridgeBuilt }, use, testInfo) => {
    void bridgeBuilt;
    const dataDir = mkdtempSync(join(tmpdir(), "atlas-e2e-"));
    const requestLog = testInfo.outputPath("bridge-requests.log");
    const bridge = await startBridge(dataDir, scene, requestLog, retryScale);
    await use(bridge);
    await bridge.stop();
    if (testInfo.status === testInfo.expectedStatus) {
      rmSync(requestLog, { force: true });
    } else {
      try {
        copyFileSync(join(dataDir, "state", "log", "atlas.log"), testInfo.outputPath("atlas.log"));
      } catch {
        // An app that never wrote its log leaves nothing to keep.
      }
    }
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
  recovery: { name: "bench-c", id: "sim-bench:SIM-REC-1" },
  offline: { name: "old-bench", id: "sim-bench:SIM-OLD-1" },
  disk: { name: "SD card 64 GB", id: "removable-disk:SIM-CARD-1" },
} as const;

/** Waits until the demo world has settled in the tree: every record found, named, and the old
 *  board gone offline. The robot's own device (the radio) is a row of the robot's screen and of
 *  the device list, not of the tree; its robot is. */
export async function demoSettled(page: Page) {
  const side = page.locator("aside.side");
  await expect(side.locator("a.nav .dot")).toHaveCount(6);
  await expect(side.locator(".sec")).toHaveText(["Devices", "Offline", "Disks", "Tools"]);
  await expect(side.getByText("Practice field", { exact: true })).toBeVisible();
  for (const record of Object.values(DEMO)) {
    if (record !== DEMO.radio) await expect(side.getByText(record.name, { exact: true })).toBeVisible();
  }
}

/** Holds the page's first snapshot back until the returned function is called: the app as it is
 *  before the core has answered. Every other call goes through. Set before the page is opened. */
export async function holdSnapshot(page: Page): Promise<() => void> {
  let release!: () => void;
  const held = new Promise<void>((done) => (release = done));
  await page.route("**/call", async (route) => {
    if ((route.request().postDataJSON() as { name?: string } | null)?.name === "snapshot") await held;
    await route.continue();
  });
  return release;
}
