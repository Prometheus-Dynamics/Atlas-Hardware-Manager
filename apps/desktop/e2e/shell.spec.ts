// F2, the shell: it opens with the last known inventory, state arrives by events, and the tree
// leads to every screen. Real pointer clicks against the real core and the simulated devices.
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { SIZES, expect, expectNoOverflow, startBridge, test } from "./bridge";

test("the simulated devices arrive by events and fill the tree", async ({ app }) => {
  const side = app.locator("aside.side");
  await expect(side.locator(".brand b")).toHaveText("Atlas");
  await expect(side.locator(".brand span")).toHaveText(/^\d{4}\.\d+\.\d+-dev/);
  // One device in every state, found by the scan with no call from the page.
  await expect(side.locator("a.nav .dot")).toHaveCount(5);
  await expect(side.locator(".sec")).toHaveText(["Devices", "Disks", "Tools"]);
  await expect(side.locator("a.nav .dot.acc")).toHaveCount(1); // the board in its boot ROM
});

test("a board that goes away moves to Offline, and comes back", async ({ app, bridge }) => {
  const side = app.locator("aside.side");
  await expect(side.locator("a.nav .dot.ok")).toHaveCount(2);
  await bridge.call("sim.set_online", { id: "simulated:SIM-RUN-1", online: false });
  await expect(side.locator(".sec")).toHaveText(["Devices", "Offline", "Disks", "Tools"]);
  await expect(side.locator("a.nav.off")).toHaveText("simulated:SIM-RUN-1");
  await bridge.call("sim.set_online", { id: "simulated:SIM-RUN-1", online: true });
  await expect(side.locator(".sec")).toHaveText(["Devices", "Disks", "Tools"]);
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
  await expect(page.locator("aside.side a.nav .dot")).toHaveCount(5);
  // Unplug one, so the saved inventory holds an offline record too.
  await first.call("sim.set_online", { id: "simulated:SIM-REC-1", online: false });
  await expect(page.locator("aside.side a.nav.off")).toHaveCount(1);
  await page.waitForTimeout(2500); // the inventory is saved at most every two seconds
  await first.stop();

  const second = await startBridge(dataDir);
  // What the core holds the moment it has opened, before the page asks: everything it knew.
  const snapshot = (await second.call("snapshot")) as { topics: { inventory: { devices: { presence: string }[] } } };
  expect(snapshot.topics.inventory.devices).toHaveLength(5);
  await page.goto(second.url);
  await expect(page.locator("aside.side a.nav .dot")).toHaveCount(5);
  await page.close();
  await second.stop();
  rmSync(dataDir, { recursive: true, force: true });
});

for (const size of SIZES) {
  test(`nothing overflows at ${size.width}x${size.height}`, async ({ app }) => {
    await app.setViewportSize(size);
    await expect(app.locator("aside.side a.nav .dot")).toHaveCount(5);
    await expectNoOverflow(app);
  });
}
