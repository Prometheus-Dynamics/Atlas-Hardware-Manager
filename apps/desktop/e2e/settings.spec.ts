// F7, settings, keys and security: by real pointer clicks against the real core.
import { mkdtempSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { DEMO, SIZES, demoSettled, expect, expectNoOverflow, startBridge, test } from "./bridge";

const open = async (page: import("@playwright/test").Page) => {
  await demoSettled(page);
  await page.locator("aside.side").getByText("Settings").click();
  await expect(page.locator("header.top h1")).toHaveText("Settings");
};
const panel = (page: import("@playwright/test").Page, name: string) => page.locator(`[data-panel="${name}"]`);

test("security is off by default, and the switch, keep-last-N, channel and theme are kept across a restart", async ({ browser }) => {
  const dataDir = mkdtempSync(join(tmpdir(), "atlas-e2e-"));
  const first = await startBridge(dataDir);
  const page = await browser.newPage();
  await page.goto(first.url);
  await open(page);
  const sw = panel(page, "security").getByRole("switch");
  await expect(sw).toHaveAttribute("aria-checked", "false");
  await expect(panel(page, "security").locator(".swrap .faint")).toHaveText("off");
  const before = await sw.boundingBox();
  await sw.click();
  await expect(sw).toHaveAttribute("aria-checked", "true");
  await expect(sw).toHaveClass(/\bon\b/);
  await expect(panel(page, "security").locator(".swrap .faint")).toHaveText("on");
  expect(await sw.boundingBox()).toEqual(before); // nothing moved
  await page.getByLabel("Keep last N slots").selectOption("5");
  await page.getByLabel("Channel").selectOption("beta");
  await page.getByLabel("Theme").selectOption("light");
  await expect(page.locator("html")).toHaveAttribute("data-theme", "light");
  await expect.poll(async () => ((await first.call("snapshot")) as { topics: { settings: unknown } }).topics.settings).toEqual({
    keep_last: 5,
    security: true,
    channel: "beta",
    theme: "light",
  });
  await first.stop();

  const second = await startBridge(dataDir);
  await page.goto(second.url);
  await open(page);
  await expect(panel(page, "security").getByRole("switch")).toHaveAttribute("aria-checked", "true");
  await expect(page.getByLabel("Keep last N slots")).toHaveValue("5");
  await expect(page.getByLabel("Channel")).toHaveValue("beta");
  await expect(page.locator("html")).toHaveAttribute("data-theme", "light");
  // And off again, at once, with no question asked.
  await panel(page, "security").getByRole("switch").click();
  await expect(panel(page, "security").getByRole("switch")).toHaveAttribute("aria-checked", "false");
  await page.close();
  await second.stop();
  rmSync(dataDir, { recursive: true, force: true });
});

test("a key file is added with its owner, shown in detail, and removed after a confirmation in place", async ({ app, bridge }) => {
  await open(app);
  const keys = panel(app, "keys");
  const rows = keys.locator("[data-key]");
  await expect(rows).toHaveCount(1); // the key built into Atlas
  await expect(rows.first()).toContainText("built into Atlas");

  // Choosing nothing changes nothing.
  await keys.getByRole("button", { name: "Add key…" }).click();
  await expect(rows).toHaveCount(1);

  await bridge.call("sim.pick_key", {});
  await keys.getByRole("button", { name: "Add key…" }).click();
  await expect(rows).toHaveCount(2);
  const mine = keys.locator('[data-key="0807060504030201"]');
  await expect(mine.locator("b")).toHaveText("Lab bench (local)");
  await expect(mine).toContainText("This computer's operator · bench@example.invalid");
  await expect(mine).toContainText("signs: dev and example builds · added by you");
  await expect(mine.locator(".mono")).toHaveText("minisign 0807…0201");
  await expect(keys.locator("[data-said]")).toHaveText("Added Lab bench (local).");

  // The same file again is refused, and says which key.
  await bridge.call("sim.pick_key", {});
  await keys.getByRole("button", { name: "Add key…" }).click();
  await expect(keys.locator("[data-said]")).toHaveText("minisign:0807060504030201 is already trusted.");
  await expect(rows).toHaveCount(2);

  // Details: everything the key file says.
  await mine.getByRole("button", { name: "Details" }).click();
  const details = panel(app, "key-details");
  await expect(details.locator("dd")).toHaveText([
    "Lab bench (local)", "This computer's operator", "bench@example.invalid", "dev and example builds",
    "minisign:0807060504030201", "2026-11-01", "never", "this computer's key list", "Lab test images only.",
  ]);
  await expect(mine).toHaveAttribute("data-open", "true");

  // Remove asks in place: the line keeps its place and height, Cancel leaves the key.
  const line = details.locator(".confirm-line");
  const before = await line.boundingBox();
  await details.getByRole("button", { name: "Remove…" }).click();
  await expect(line.locator(".q")).toHaveText("Remove Lab bench (local) from the trusted keys? Files it signed stop being trusted.");
  const asking = await line.boundingBox();
  expect([asking!.y, asking!.height]).toEqual([before!.y, before!.height]);
  await details.getByRole("button", { name: "Cancel" }).click();
  await expect(rows).toHaveCount(2);
  await details.getByRole("button", { name: "Remove…" }).click();
  await details.getByRole("button", { name: "Remove key" }).click();
  await expect(rows).toHaveCount(1);
  await expect(panel(app, "checks")).toBeVisible();

  // The built-in key has details and no Remove.
  await rows.first().getByRole("button", { name: "Details" }).click();
  await expect(panel(app, "key-details").locator("dd").nth(7)).toHaveText("built in");
  await expect(panel(app, "key-details").getByRole("button")).toHaveCount(0);
});

test("a file that is not a key file is refused in the library's words", async ({ app, bridge }) => {
  await open(app);
  const keys = panel(app, "keys");
  await bridge.call("sim.pick_key", { broken: true });
  await keys.getByRole("button", { name: "Add key…" }).click();
  await expect(keys.locator("[data-said]")).toHaveText(/^notes\.json: key must be a string/);
  await expect(keys.locator("[data-key]")).toHaveCount(1);
});

test("the switch decides what an unsigned image is: used when off, refused when on, and both checks are recorded", async ({ app, bridge }) => {
  await open(app);
  await expect(panel(app, "checks")).toContainText("Nothing has been checked yet.");
  const side = app.locator("aside.side");

  // Off: the unsigned sample image is read and is ready.
  await bridge.call("sim.pick_image", {});
  await side.getByText("Flash a board").click();
  await app.getByRole("button", { name: "Choose image…" }).click();
  await expect(app.locator('[data-panel="image"]')).toHaveAttribute("data-image", "ready");
  await side.getByText("Settings").click();
  const rows = panel(app, "checks").locator(".tr");
  await expect(rows).toHaveCount(1);
  await expect(rows.first().locator("div")).toHaveText([/^\d\d:\d\d:\d\d$/, "image to flash", "demo-os-2026.11.2.img", "unsigned · used: security is off"]);

  // On: the same file is refused before anything is sent to a board.
  await panel(app, "security").getByRole("switch").click();
  await bridge.call("sim.pick_image", {});
  await side.getByText("Flash a board").click();
  await app.getByRole("button", { name: "Choose another…" }).click();
  await expect(app.locator('[data-panel="image"]')).toHaveAttribute("data-image", "refused");
  await expect(app.locator('[data-panel="image"]')).toContainText("Security is on, and this file is unsigned");
  await side.getByText("Settings").click();
  await expect(rows).toHaveCount(2);
  await expect(rows.first().locator("div").nth(3)).toHaveText("unsigned · refused");
});

test("a support bundle is a file on this computer, and the page says what it holds", async ({ app, bridge }) => {
  await open(app);
  const support = panel(app, "support");
  await expect(support.locator("dd").first()).toHaveText(/state[\\/]log[\\/]atlas\.log$/);
  await expect(support.locator("dd").nth(1)).toHaveText(/^[\d.]+ (B|KB|MB) · UI errors, failed calls, breadcrumbs$/);
  await support.getByRole("button", { name: "Make support bundle" }).click();
  const line = support.locator("[data-bundle]");
  // Where the bundle is, in full and on the page, with what it holds.
  await expect(line).toHaveText(/^\/.*atlas-support-\d+\.txt · [\d.]+ (KB|MB) · holds: Atlas, Settings, Devices and recent events, Scan, Jobs, App log: atlas\.log/);
  await expect(line).not.toHaveAttribute("title");
  expect(await line.evaluate((el) => el.scrollWidth <= el.clientWidth)).toBe(true);
  // The file is there, readable, and holds what the page says.
  const path = (await line.textContent())!.trim().split(" · ")[0]!;
  const text = readFileSync(path, "utf8");
  for (const part of ["===== Atlas =====", "===== Settings =====", "===== Devices and recent events =====", "===== App log: atlas.log ====="]) expect(text).toContain(part);
  expect(text).toContain(DEMO.running.name);
  expect(text).toContain("support bundle asked for");

  await support.getByRole("button", { name: "Open log folder" }).click();
  await support.getByRole("button", { name: "Show the bundle" }).click();
  await expect.poll(async () => ((await bridge.call("sim.opened")) as string[]).map((p) => p.replace(/^.*[\\/]state[\\/]/, ""))).toEqual(["log", "support"]);
});

test("a robot is added by name and team, a bad team is refused, and it is removed at once", async ({ app }) => {
  await open(app);
  const robots = panel(app, "robots");
  await expect(robots.locator("[data-robot]")).toHaveText(["Practice field team 9999 Remove"]);
  await robots.getByLabel("Robot name").fill("Comp bot");
  await robots.getByLabel("Team number").fill("seventy");
  await robots.getByRole("button", { name: "Add robot" }).click();
  await expect(robots.locator("[data-robot-said]")).toHaveText("A team is a number from 1 to 25599.");
  await robots.getByLabel("Team number").fill("4242");
  await robots.getByRole("button", { name: "Add robot" }).click();
  await expect(robots.locator("[data-robot]")).toHaveCount(2);
  await expect(app.locator("aside.side").getByText("Comp bot", { exact: true })).toBeVisible();
  await expect(robots.getByLabel("Robot name")).toHaveValue("");

  // Removed at once, with no question: it can be added again.
  await robots.locator('[data-robot="Comp bot"]').getByRole("button", { name: "Remove" }).click();
  await expect(robots.locator("[data-robot]")).toHaveCount(1);
  await expect(app.locator("aside.side").getByText("Comp bot", { exact: true })).toHaveCount(0);
});

for (const size of SIZES) {
  test(`settings do not overflow at ${size.width}x${size.height}`, async ({ app, bridge }) => {
    await app.setViewportSize(size);
    await open(app);
    await expectNoOverflow(app);
    await bridge.call("sim.pick_key", {});
    await panel(app, "keys").getByRole("button", { name: "Add key…" }).click();
    await panel(app, "keys").locator('[data-key="0807060504030201"]').getByRole("button", { name: "Details" }).click();
    await panel(app, "key-details").getByRole("button", { name: "Remove…" }).click();
    await panel(app, "support").getByRole("button", { name: "Make support bundle" }).click();
    await expect(panel(app, "support").locator("[data-bundle]")).toBeVisible();
    await expectNoOverflow(app);
  });
}
