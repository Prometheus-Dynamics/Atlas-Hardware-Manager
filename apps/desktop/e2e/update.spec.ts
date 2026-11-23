// F9, the app updating itself: by real pointer clicks against the real core, the real
// atlas-release checks, and the simulated release feed (signed in the simulated world).
import { readFileSync } from "node:fs";
import { join } from "node:path";
import type { Page } from "@playwright/test";
import { SIZES, demoSettled, expect, expectNoOverflow, test } from "./bridge";

const open = async (page: Page) => {
  await demoSettled(page);
  await page.locator("aside.side").getByText("Settings").click();
  await expect(page.locator("header.top h1")).toHaveText("Settings");
  // The support panel's log path is a one-line bar until the core answers, then wraps to its real
  // height: panels are measured only after that, or a slow answer reads as a moved panel.
  await expect(page.locator('[data-panel="support"] .ph-skel')).toHaveCount(0);
};
const controls = (page: Page) => page.locator("[data-update-controls]");
const line = (page: Page) => page.locator("[data-update]");
const check = (page: Page) => page.getByRole("button", { name: "Check for updates" });

test("with no update source there is nothing to press, and the page says so", async ({ app }) => {
  await open(app);
  await expect(controls(app)).toHaveText("no update source configured");
  await expect(controls(app).locator("button")).toHaveCount(0);
  await expect(app.locator("[data-update-checked]")).toHaveText("");
});

test("a check that finds nothing says up to date and when it was checked, and moves nothing", async ({ app, bridge }) => {
  await open(app);
  await bridge.call("sim.feed", { state: "nothing", kind: "deb" });
  await expect(app.locator("[data-update-checked]")).toHaveText("not checked yet");
  const below = app.locator('[data-panel="support"]');
  const before = await below.boundingBox();
  await check(app).click();
  await expect(line(app)).toHaveText("up to date");
  await expect(app.locator("[data-update-checked]")).toHaveText("checked just now");
  await expect(check(app)).toBeVisible();
  expect(await below.boundingBox()).toEqual(before);
});

test("a newer signed release is offered, downloaded with progress, cancelled, resumed and handed to the system's installer", async ({ app, bridge }) => {
  await open(app);
  await bridge.call("sim.feed", { state: "newer", kind: "deb", hold: true });
  const below = app.locator('[data-panel="support"]');
  const before = await below.boundingBox();
  await check(app).click();
  await expect(line(app)).toHaveText("9999.1.0 is available · 393 KB");
  await app.getByRole("button", { name: "Download" }).click();
  // Held after its first pieces: a quarter of it has arrived, and it says so.
  await expect(line(app).locator(".bar i")).toHaveAttribute("style", "width: 25%;");
  await expect(line(app)).toContainText("25% · 98.3 KB of 393 KB");
  expect(await below.boundingBox()).toEqual(before);
  await app.getByRole("button", { name: "Cancel" }).click();
  await expect(app.getByRole("button", { name: "Download" })).toBeVisible();
  await bridge.call("sim.feed", { hold: false });
  await app.getByRole("button", { name: "Download" }).click();
  await expect(line(app)).toHaveText("9999.1.0 is downloaded and checked");
  await app.getByRole("button", { name: "Open installer" }).click();
  await expect(line(app)).toHaveText("9999.1.0: handed to the system's installer");
  const opened = (await bridge.call("sim.opened")) as string[];
  expect(opened.at(-1)).toMatch(/updates\/atlas-hardware-manager_9999\.1\.0\.deb$/);
  expect(readFileSync(opened.at(-1)!).length).toBe(384 * 1024);
  expect(await below.boundingBox()).toEqual(before);
  // The key that signed it is in the trusted list, like any other.
  await expect(app.locator('[data-panel="keys"]')).toContainText("Simulated release key");
});

test("an AppImage is replaced in place", async ({ app, bridge }) => {
  await open(app);
  await bridge.call("sim.feed", { state: "newer", kind: "app-image" });
  await check(app).click();
  await app.getByRole("button", { name: "Download" }).click();
  await app.getByRole("button", { name: "Replace this AppImage" }).click();
  await expect(line(app)).toHaveText("replaced with 9999.1.0: start Atlas again to run it");
  expect(readFileSync(join(bridge.dataDir, "data/sim-files/Atlas.AppImage")).length).toBe(384 * 1024);
});

test("a release no trusted key signed is refused in the library's words, whatever the security switch says", async ({ app, bridge }) => {
  await open(app);
  await bridge.call("sim.feed", { state: "stranger", kind: "deb" });
  await expect(app.locator('[data-panel="security"]').getByRole("switch")).toHaveAttribute("aria-checked", "false");
  await check(app).click();
  await expect(line(app)).toHaveText("the manifest for atlas-app 9999.1.0 has no valid signature from a trusted key");
  await expect(app.getByRole("button", { name: "Download" })).toHaveCount(0);
  await expect(check(app)).toBeVisible();
});

test("a feed that is down is asked again until it is stopped by hand, and answers when it is back", async ({ app, bridge }) => {
  await open(app);
  await bridge.call("sim.feed", { state: "down", kind: "deb" });
  await check(app).click();
  await expect(controls(app).locator(".chip")).toHaveText("Retrying");
  await expect(line(app)).toContainText("(try 2)");
  await expect(line(app)).toContainText("connection refused");
  // Back while it is still asking: the next try finds the release with no click.
  await bridge.call("sim.feed", { state: "newer" });
  await expect(line(app)).toContainText("9999.1.0 is available");
  // Down again, and stopped: nothing is asked until the person checks.
  await bridge.call("sim.feed", { state: "down" });
  await check(app).click();
  await app.getByRole("button", { name: "Stop" }).click();
  await expect(check(app)).toBeVisible();
  await expect(line(app)).toHaveText("");
  await bridge.call("sim.feed", { state: "nothing" });
  await app.waitForTimeout(1500);
  await expect(line(app)).toHaveText("");
});

for (const size of SIZES) {
  test(`nothing overflows with an update downloading at ${size.width}x${size.height}`, async ({ app, bridge }) => {
    await app.setViewportSize(size);
    await open(app);
    await bridge.call("sim.feed", { state: "newer", kind: "deb", hold: true });
    await check(app).click();
    await expectNoOverflow(app);
    await app.getByRole("button", { name: "Download" }).click();
    await expect(line(app).locator(".bar")).toBeVisible();
    await expectNoOverflow(app);
  });
}
