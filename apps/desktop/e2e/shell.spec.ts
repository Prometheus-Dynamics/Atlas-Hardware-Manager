// F2, the shell: it opens with the last known inventory, state arrives by events, and the tree
// leads to every screen. Real pointer clicks against the real core and the simulated devices.
import { mkdtempSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { DEMO, SIZES, demoSettled, expect, expectNoOverflow, holdSnapshot, startBridge, test } from "./bridge";

test("the simulated devices arrive by events and fill the tree", async ({ app }) => {
  const side = app.locator("aside.side");
  await expect(side.locator(".brand b")).toHaveText("Atlas");
  await expect(side.locator(".brand span")).toHaveText(/^\d{4}\.\d+\.\d+-dev/);
  // One device in every state, found by the scan with no call from the page.
  await demoSettled(app);
  await expect(side.locator("a.nav .dot.acc")).toHaveCount(1); // the board in its boot ROM
  // The robot, its controller under it, then the boards on this computer.
  await expect(side.locator("a.nav .t")).toHaveText([
    "All devices", "Practice field", "sc-practice", "bench-a", "bench-b", "bench-c", "old-bench", "SD card 64 GB", "Flash a board", "Settings",
  ]);
});

test("a board that goes away moves to Offline, and comes back", async ({ app, bridge }) => {
  const side = app.locator("aside.side");
  await demoSettled(app);
  await expect(side.locator("a.nav.off")).toHaveText(["old-bench"]);
  await bridge.call("sim.set_online", { id: DEMO.recovery.id, online: false });
  await expect(side.locator("a.nav.off")).toHaveText(["bench-c", "old-bench"]);
  await bridge.call("sim.set_online", { id: DEMO.recovery.id, online: true });
  await expect(side.locator("a.nav.off")).toHaveText(["old-bench"]);
});

test("the tree leads to each screen, and the selected row shows it", async ({ app }) => {
  const side = app.locator("aside.side");
  await expect(side.locator("a.nav.on")).toHaveText("All devices");
  await side.getByText("Settings").click();
  await expect(app.locator("header.top h1")).toHaveText("Settings");
  await expect(side.locator("a.nav.on")).toHaveText("Settings");
  await side.getByText("Flash a board").click();
  await expect(app.locator("header.top h1")).toHaveText("Flash a board");
  await app.goBack();
  await expect(app.locator("header.top h1")).toHaveText("Settings");
  await side.getByText("All devices").click();
  await expect(app.locator("header.top h1")).toHaveText("Devices");
});

test("it opens with the last known inventory before any scan answers", async ({ browser }) => {
  const dataDir = mkdtempSync(join(tmpdir(), "atlas-e2e-"));
  const first = await startBridge(dataDir);
  const page = await browser.newPage();
  await page.goto(first.url);
  await demoSettled(page);
  // The inventory is saved at most every two seconds: wait until the saved file holds every device.
  await expect
    .poll(() => {
      try {
        return (JSON.parse(readFileSync(join(dataDir, "data", "inventory.json"), "utf8")) as { devices: unknown[] }).devices.length;
      } catch {
        return 0;
      }
    }, { timeout: 10_000 })
    .toBe(7);
  await first.stop();

  const second = await startBridge(dataDir);
  // What the core holds the moment it has opened, before the page asks: everything it knew.
  const snapshot = (await second.call("snapshot")) as { topics: { inventory: { devices: { presence: string }[] } } };
  expect(snapshot.topics.inventory.devices).toHaveLength(7);
  await page.goto(second.url);
  await demoSettled(page);
  await page.close();
  await second.stop();
  rmSync(dataDir, { recursive: true, force: true });
});

test("before the core's first answer every screen shows bars, and none says there is nothing", async ({ page, bridge }) => {
  const release = await holdSnapshot(page);
  await page.goto(bridge.url);
  const side = page.locator("aside.side");
  await expect(side.locator("[data-loading] .ph-skel")).toHaveCount(2);
  await expect(side).not.toContainText("No devices yet");
  const NONE = /No devices yet|Nothing found yet|no record of this board|There is no robot named|knows no controller|none chosen yet|No board is in its boot ROM|No removable disk|Nothing has been|None yet|None seen/;
  const screens = ["#/devices", `#/board/${encodeURIComponent(DEMO.running.id)}`, "#/robot/Practice%20field", `#/systemcore/${encodeURIComponent(DEMO.controller.id)}`, "#/flash", "#/disk", "#/settings"];
  for (const hash of screens) {
    await page.evaluate((to) => (location.hash = to), hash);
    await expect.poll(() => page.evaluate(() => location.hash), { message: hash }).toBe(hash);
    await expect(page.locator(".main .ph-skel").first(), hash).toBeVisible();
    await expect(page.locator(".main"), hash).not.toContainText(NONE);
    await expect(page.locator(".main .empty-box"), hash).toHaveCount(0);
  }
  // A switch with no value yet is the disabled one, and cannot be pressed.
  const security = page.locator('[data-panel="security"]').getByRole("switch");
  await expect(security).toHaveClass(/\bdis\b/);
  await expect(security).toBeDisabled();
  release();
  await demoSettled(page);
  await expect(page.locator("[data-loading]")).toHaveCount(0);
  await expect(security).not.toHaveClass(/\bdis\b/);
  await expect(security).toBeEnabled();
});

test("a core that cannot be reached is said in words, and the screen fills in when it answers", async ({ page, bridge }) => {
  await page.route("**/call", (route) => route.abort());
  await page.goto(bridge.url);
  const note = page.locator('[data-panel="unreachable"] .note.err');
  await expect(note).toContainText("Atlas cannot reach its own core");
  await expect(note).toContainText("It keeps trying");
  await expect(page.locator("aside.side")).not.toContainText("No devices yet");
  await expect(page.locator(".main")).not.toContainText("Nothing found yet");
  await page.unroute("**/call");
  await demoSettled(page);
  await expect(page.locator('[data-panel="unreachable"]')).toHaveCount(0);
  await expect(page.locator(".tr.dev")).toHaveCount(7);
});

test("a robot's own devices are rows of its screen, not of the tree", async ({ app }) => {
  const side = app.locator("aside.side");
  await demoSettled(app);
  await expect(app.locator(".tr.dev")).toHaveCount(7);
  await expect(side.getByText(DEMO.radio.name, { exact: true })).toHaveCount(0);
  await side.getByText(DEMO.controller.name, { exact: true }).click();
  await expect(side.locator("a.nav.on")).toHaveText(DEMO.controller.name);
  await side.getByText("Practice field", { exact: true }).click();
  await expect(app.locator(`[data-robot-device="${DEMO.radio.id}"]`)).toBeVisible();
});

test("on a short window the devices scroll inside the rail and the tools stay in reach", async ({ app }) => {
  await demoSettled(app);
  await app.setViewportSize({ width: 1024, height: 240 });
  const side = app.locator("aside.side");
  const settings = side.getByText("Settings", { exact: true });
  await expect(settings).toBeInViewport({ ratio: 1 });
  expect(await side.locator("[data-tree]").evaluate((el) => el.scrollHeight > el.clientHeight)).toBe(true);
  await settings.click();
  await expect(app.locator("header.top h1")).toHaveText("Settings");
});

for (const size of SIZES) {
  test(`nothing overflows at ${size.width}x${size.height}`, async ({ app }) => {
    await app.setViewportSize(size);
    await demoSettled(app);
    await expectNoOverflow(app);
  });
}
