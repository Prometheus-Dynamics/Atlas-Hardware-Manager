// F2, the shell: it opens with the last known inventory, state arrives by events, and the tree
// leads to every screen. Real pointer clicks against the real core and the simulated devices.
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { DEMO, SIZES, demoSettled, expect, expectNoOverflow, startBridge, test } from "./bridge";

test("the simulated devices arrive by events and fill the tree", async ({ app }) => {
  const side = app.locator("aside.side");
  await expect(side.locator(".brand b")).toHaveText("Atlas");
  await expect(side.locator(".brand span")).toHaveText(/^\d{4}\.\d+\.\d+-dev/);
  // One device in every state, found by the scan with no call from the page.
  await demoSettled(app);
  await expect(side.locator("a.nav .dot.acc")).toHaveCount(1); // the board in its boot ROM
  // The robot, its controller under it, then the boards on this computer.
  await expect(side.locator("a.nav .t")).toHaveText([
    "All devices", "Practice field", "sc-practice", "field-radio", "bench-a", "bench-b", "bench-c", "old-bench", "SD card 64 GB", "Flash a board", "Settings",
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
  await page.waitForTimeout(2500); // the inventory is saved at most every two seconds
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

for (const size of SIZES) {
  test(`nothing overflows at ${size.width}x${size.height}`, async ({ app }) => {
    await app.setViewportSize(size);
    await demoSettled(app);
    await expectNoOverflow(app);
  });
}
