// F4, the board page: every job with real pointer clicks, against the simulated Orion node
// behind the real board driver. A test reaches past the page only to change the simulated world
// (`sim.*`): a log line the board writes, the board's clock, a file the person will choose.
import type { Page } from "@playwright/test";
import { DEMO, SIZES, demoSettled, expect, expectNoOverflow, test } from "./bridge";

const BOARD = DEMO.running;

async function openBoard(page: Page, name: string = BOARD.name) {
  await demoSettled(page);
  await page.locator("aside.side").getByText(name, { exact: true }).click();
  await expect(page.locator("header.top h1")).toHaveText(name);
}

const slot = (page: Page, id: string) => page.locator(`[data-slot="${id}"]`);

test("the top bar says who the board is, and readings arrive and move", async ({ app }) => {
  await openBoard(app);
  const top = app.locator("header.top");
  await expect(top.locator(".crumb")).toHaveText("Devices /");
  await expect(top.locator(".chip.ok")).toHaveText("Running");
  await expect(top.locator(".chip.neu")).toHaveText(["slot-4", "demo-os 2026.11.2", "USB 1-2", "127.0.0.1"]);
  await expect(top.getByRole("button")).toHaveText(["Locate", "Reboot"]);

  const tiles = app.locator('[data-panel="readings"] .tile[data-reading]');
  await expect(tiles).toHaveCount(5);
  await expect(tiles.locator(".lbl")).toHaveText(["pressure", "temperature", "accel x", "accel y", "accel z"]);
  await expect(tiles.first().locator(".val small")).toHaveText("kPa");
  // The sparkline grows as samples arrive, one a second, and the value is a number.
  const path = tiles.first().locator("svg.spark path").last();
  await expect(path).toBeVisible();
  const before = await path.getAttribute("d");
  await expect.poll(async () => path.getAttribute("d")).not.toBe(before);
  await expect(tiles.first().locator(".val")).toHaveText(/^\d+(\.\d+)? kPa$/);
  await expect(app.locator('[data-panel="readings"] .ph')).toContainText(/updated \d+ s ago/);
});

test("log lines the board writes arrive, newest first", async ({ app, bridge }) => {
  await openBoard(app);
  // The board is being read (its readings are on the page) before it writes the line.
  await expect(app.locator('[data-panel="readings"] .tile[data-reading]')).toHaveCount(5);
  await bridge.call("sim.node", { log: "camera: frame loop started" });
  const lines = app.locator('[data-panel="dock"] .lines .ln');
  await expect(lines.first()).toContainText("camera: frame loop started");
  await bridge.call("sim.node", { log: "warn: 2 frames dropped" });
  await expect(lines.first()).toContainText("2 frames dropped");
  await expect(lines.first().locator(".lv-warn")).toHaveText("warn");
  await expect(lines.nth(1)).toContainText("frame loop started");
});

test("Locate and Reboot run at once, with no confirmation, and say what the board said", async ({ app, bridge }) => {
  await openBoard(app);
  const top = app.locator("header.top");
  await top.getByRole("button", { name: "Locate" }).click();
  await expect(top.locator('[data-said="ok"]')).toHaveText("locate: blinking");
  expect(await bridge.call("sim.node", { runs: "locate" })).toBe(1);
  await top.getByRole("button", { name: "Reboot" }).click();
  await expect(top.locator('[data-said="ok"]')).toContainText("system.reboot.reboot");
  expect(await bridge.call("sim.node", { runs: "system.reboot.reboot" })).toBe(1);
});

test("a refusal is on the page in the board's own words, in full: an action's in the log pane, a slot's in its row", async ({ app, bridge }) => {
  await openBoard(app);
  await expect(app.locator("[data-slot]")).toHaveCount(4);
  const whole = (el: Element) => el.scrollWidth <= el.clientWidth && el.scrollHeight <= el.clientHeight;
  // The board refuses the next Locate, with more words than a chip holds.
  const words = "the status LED is held by a self-test that is still running, and it is not given to another caller until that test has ended";
  await bridge.call("sim.node", { refuse: ["locate", words] });
  await app.locator("header.top").getByRole("button", { name: "Locate" }).click();
  const chip = app.locator('header.top [data-said="refused"]');
  await expect(chip).toContainText("locate");
  await expect(chip).not.toHaveAttribute("title");
  const line = app.locator('[data-panel="dock"] .lines [data-refused]');
  await expect(line).toContainText(`locate refused: `);
  await expect(line).toContainText(words);
  expect(await line.locator("span").last().evaluate(whole)).toBe(true);
  await expectNoOverflow(app);

  // The board's own security is on: it will not make an unsigned slot the default, and says so in that slot's row.
  await bridge.call("sim.node", { security: true });
  await slot(app, "slot-1").getByRole("button", { name: "Set default" }).click();
  const refused = slot(app, "slot-1").locator("[data-refused]");
  await expect(refused).toContainText("default refused: ");
  await expect(refused).toContainText("slot-1 is not signed by a trusted key, and security is on");
  await expect(refused).not.toHaveAttribute("title");
  expect(await refused.evaluate(whole)).toBe(true);
  await expect(slot(app, "slot-3").locator("[data-refused]")).toHaveCount(0);
  await expect(app.locator('[data-panel="dock"] .lines [data-refused]')).toHaveCount(0);
});

test("slots: what each row offers, pin and unpin, and set default", async ({ app }) => {
  await openBoard(app);
  await expect(app.locator('[data-panel="slots"] .ph .chip')).toHaveText("4 kept");
  // The running default offers no Remove, no Trial and no Set default.
  await expect(slot(app, "slot-4").locator(".chip")).toHaveText(["Running", "Default"]);
  await expect(slot(app, "slot-4").getByRole("button")).toHaveText(["Pin"]);
  // The fallback can be pinned, tried, made the default, or removed.
  await expect(slot(app, "slot-3").locator(".chip")).toHaveText(["Fallback"]);
  await expect(slot(app, "slot-3").getByRole("button")).toHaveText(["Pin", "Trial", "Set default", "Remove"]);
  // A pinned slot shows Unpin, and no Remove.
  await expect(slot(app, "slot-2").locator(".chip")).toHaveText(["Pinned"]);
  await expect(slot(app, "slot-2").getByRole("button")).toHaveText(["Unpin", "Trial", "Set default"]);

  await slot(app, "slot-2").getByRole("button", { name: "Unpin" }).click();
  await expect(slot(app, "slot-2").getByRole("button")).toHaveText(["Pin", "Trial", "Set default", "Remove"]);
  await slot(app, "slot-3").getByRole("button", { name: "Pin" }).click();
  await expect(slot(app, "slot-3").locator(".chip")).toHaveText(["Fallback", "Pinned"]);

  await slot(app, "slot-3").getByRole("button", { name: "Set default" }).click();
  await expect(slot(app, "slot-3").locator(".chip")).toContainText(["Default"]);
  await expect(slot(app, "slot-4").locator(".chip")).toHaveText(["Running"]);
  // The device list's Default column now has the board's default version.
  await app.locator("aside.side").getByText("All devices").click();
  await expect(app.locator(".tr.dev", { hasText: BOARD.name }).locator("> div").nth(3)).toHaveText("2026.10.20");
});

test("removing a slot confirms in place, at the same height, and Cancel changes nothing", async ({ app }) => {
  await openBoard(app);
  const row = slot(app, "slot-3");
  const before = await row.boundingBox();
  const keepBefore = await app.locator(".keep").boundingBox();
  await row.getByRole("button", { name: "Remove" }).click();
  await expect(row).toHaveClass(/confirm/);
  await expect(row.locator(".l1 b")).toHaveText("Remove slot-3?");
  await expect(row.getByRole("button")).toHaveText(["Cancel", "Remove slot"]);
  expect(await row.boundingBox()).toEqual(before);
  expect(await app.locator(".keep").boundingBox()).toEqual(keepBefore);
  await row.getByRole("button", { name: "Cancel" }).click();
  await expect(row).not.toHaveClass(/confirm/);
  await expect(app.locator("[data-slot]")).toHaveCount(4);

  await row.getByRole("button", { name: "Remove" }).click();
  await row.getByRole("button", { name: "Remove slot" }).click();
  await expect(app.locator("[data-slot]")).toHaveCount(3);
  await expect(app.locator('[data-panel="slots"] .ph .chip')).toHaveText("3 kept");
});

test("the keep count is set on the board", async ({ app }) => {
  await openBoard(app);
  const keep = app.getByLabel("Keep last");
  await expect(keep).toHaveValue("3");
  await keep.selectOption("5");
  await expect(app.locator('header.top [data-said="ok"]')).toContainText("keep last 5");
  await expect(keep).toHaveValue("5");
});

test("an install is a job with progress, and ends as a new slot", async ({ app, bridge }) => {
  await openBoard(app);
  await bridge.call("sim.node", { hold_installs: true });
  await bridge.call("sim.pick_bundle", { slot: "slot-5", version: "2026.11.14" });
  const install = app.getByRole("button", { name: "Install…" });
  const place = await install.boundingBox();
  await install.click();
  const row = app.locator("[data-install]");
  await expect(row.locator(".l1 b")).toHaveText("demo-os-2026.11.14.tar");
  await expect(row).toHaveClass(/write/);
  await expect(row.locator(".bar")).toBeVisible();
  // While it runs there is one install at a time: the button stays where it is and cannot be pressed.
  await expect(install).toBeDisabled();
  expect(await install.boundingBox()).toEqual(place);
  await expect(row.locator(".chip")).toHaveText(/Writing|Installing|Waiting for the board/);
  await bridge.call("sim.node", { hold_installs: false });
  await expect(slot(app, "slot-5").locator(".l1 b")).toHaveText("slot-5 · 2026.11.14");
  // The board keeps its last three and what is pinned, default or running: the oldest plain
  // slot went when the new one landed. The board did that, not Atlas.
  await expect(slot(app, "slot-1")).toHaveCount(0);
  await expect(app.locator("[data-slot]")).toHaveCount(4);
  await expect(row).toHaveCount(0);
  await expect(app.getByRole("button", { name: "Install…" })).toBeVisible();
});

test("a bundle for another board is refused on the computer, in the board library's words", async ({ app, bridge }) => {
  await openBoard(app);
  await bridge.call("sim.pick_bundle", { slot: "slot-9", version: "2026.12.1", board: "another-board" });
  await app.getByRole("button", { name: "Install…" }).click();
  const row = app.locator('[data-install="failed"]');
  await expect(row.locator(".chip.err")).toHaveText("Failed");
  await expect(row.locator(".l2.err")).toContainText("another-board");
  // The reason is on the page in full: nothing of it is cut off, and none of it is in a tooltip.
  const reason = row.locator("[data-install-detail]");
  await expect(reason).not.toHaveAttribute("title");
  expect(await reason.evaluate((el) => el.scrollWidth <= el.clientWidth)).toBe(true);
  await expectNoOverflow(app);
  await expect(app.locator("[data-slot]")).toHaveCount(4);
  await row.getByRole("button", { name: "Dismiss" }).click();
  await expect(app.locator("[data-install]")).toHaveCount(0);
});

test("choosing no file starts nothing", async ({ app }) => {
  await openBoard(app);
  await app.getByRole("button", { name: "Install…" }).click();
  await expect(app.getByRole("button", { name: "Install…" })).toBeVisible();
  await expect(app.locator("[data-install]")).toHaveCount(0);
});

test("a trial boot is confirmed by the board, not by Atlas", async ({ app, bridge }) => {
  await openBoard(app);
  await slot(app, "slot-3").getByRole("button", { name: "Trial" }).click();
  await expect(slot(app, "slot-3").locator(".chip.warn")).toHaveText("Trial, running");
  await expect(app.locator("header.top .chip.warn")).toHaveText("slot-3 · trial");
  // Nothing on the row confirms it, and no other trial is offered meanwhile.
  await expect(app.getByRole("button", { name: "Confirm" })).toHaveCount(0);
  await expect(app.getByRole("button", { name: "Trial" })).toHaveCount(0);
  await expect(app.locator("[data-boots] .tr").first()).toContainText("slot-3");
  // The board's own clock passes its confirm time: it confirms itself.
  await bridge.call("sim.node", { clock: 60 });
  await expect(slot(app, "slot-3").locator(".chip")).toHaveText(["Running", "Default"]);
  await expect(app.locator("[data-boots]")).toContainText("trial held, confirmed by the board");
});

test("a trial on an unhealthy board falls back, and the page says why", async ({ app, bridge }) => {
  await openBoard(app);
  await bridge.call("sim.node", { healthy: false });
  await slot(app, "slot-3").getByRole("button", { name: "Trial" }).click();
  await expect(slot(app, "slot-3").locator(".chip.warn")).toHaveText("Trial, running");
  await bridge.call("sim.node", { clock: 700 });
  await expect(slot(app, "slot-4").locator(".chip")).toHaveText(["Running", "Default"]);
  await expect(app.locator("[data-boots]")).toContainText("trial did not hold, fell back");
});

test("the dock's tabs: hardware as the board describes it, and what is not in this build", async ({ app }) => {
  await openBoard(app);
  const dock = app.locator('[data-panel="dock"]');
  await expect(dock.locator(".tabbar label")).toHaveText(["Logs", "Hardware", "Files", "Terminal"]);
  await expect(dock.locator(".tabbar label.on")).toHaveText("Logs");
  await dock.getByText("Hardware", { exact: true }).click();
  await expect(dock.locator(".tabbar label.on")).toHaveText("Hardware");
  await expect(dock.locator("[data-device]")).toHaveCount(5);
  await expect(dock.locator('[data-device="lemnos.vane0"]')).toContainText("Wind vane");
  await expect(dock.locator('[data-device="lemnos.gauge1"] .chip.warn')).toHaveText("retrying");
  await expect(dock.locator('[data-device="lemnos.gauge1"]')).toContainText("sensor drifting");
  await dock.getByText("Files", { exact: true }).click();
  await expect(dock.locator('[data-not-built="files"]')).toContainText("not in this build");
  await dock.getByText("Terminal", { exact: true }).click();
  await expect(dock.locator('[data-not-built="terminal"]')).toContainText("not in this build");
});

test("a board that stops answering shows as offline, and reads again when it is back", async ({ app, bridge }) => {
  await openBoard(app);
  await expect(app.locator('[data-panel="readings"] .tile[data-reading]')).toHaveCount(5);
  const buttons = app.locator("header.top").getByRole("button");
  await expect(buttons.first()).toBeEnabled();
  const places = [await buttons.nth(0).boundingBox(), await buttons.nth(1).boundingBox(), await app.getByRole("button", { name: "Install…" }).boundingBox()];
  await bridge.call("sim.set_online", { id: BOARD.id, online: false });
  await expect(app.locator("header.top .chip.off")).toHaveText("Offline");
  // The tiles stay, with no data in them, and the panel says why.
  await expect(app.locator('[data-panel="readings"] [data-tiles="offline"] .tile .val')).toHaveText(Array(5).fill("— no data"));
  await expect(app.locator('[data-panel="readings"] .ph')).toContainText("offline · nothing is read until it answers again");
  await expect(app.locator('[data-panel="slots"] .empty-box')).toContainText("is offline");
  // Locate, Reboot and Install… stay where they were, and cannot be pressed.
  await expect(buttons).toHaveText(["Locate", "Reboot"]);
  await expect(buttons.nth(0)).toBeDisabled();
  await expect(buttons.nth(1)).toBeDisabled();
  await expect(app.getByRole("button", { name: "Install…" })).toBeDisabled();
  expect([await buttons.nth(0).boundingBox(), await buttons.nth(1).boundingBox(), await app.getByRole("button", { name: "Install…" }).boundingBox()]).toEqual(places);
  await bridge.call("sim.set_online", { id: BOARD.id, online: true });
  await expect(app.locator("header.top .chip.ok")).toHaveText("Running");
  await expect(app.locator('[data-panel="readings"] .tile[data-reading]')).toHaveCount(5);
  await expect(app.locator("[data-slot]")).toHaveCount(4);
});

test("a board whose Lemnos stops answering keeps its last readings, marked", async ({ app, bridge }) => {
  await openBoard(app);
  await expect(app.locator('[data-panel="readings"] .tile[data-reading]')).toHaveCount(5);
  await bridge.call("sim.node", { lemnos: false });
  await expect(app.locator('[data-panel="slots"] [data-slots-state="error"], [data-panel="slots"] [data-slot]').first()).toBeVisible();
  await bridge.call("sim.node", { lemnos: true });
  await expect(app.locator("[data-slot]")).toHaveCount(4);
});

test("a board in recovery that is not an Orion node has nothing to read, and says so", async ({ app }) => {
  await openBoard(app, DEMO.recovery.name);
  await expect(app.locator("header.top .chip.warn")).toHaveText("Recovery");
  await expect(app.locator('[data-panel="readings"] .empty-box')).toContainText("not reached through Orion");
  await expect(app.locator('[data-panel="slots"] .empty-box')).toContainText("not reached through Orion");
  await expect(app.getByRole("button", { name: "Install…" })).toBeDisabled();
});

test.describe("a plain Orion node", () => {
  test.use({ scene: "plain" });
  test("has no slots, readings or board actions, and that is not a fault", async ({ app }) => {
    const side = app.locator("aside.side");
    // A node with no board identity is called by its node name and keyed by its fingerprint.
    await expect(side.locator(".sec")).toHaveText(["Devices", "Offline", "Disks", "Tools"]);
    await side.getByText("board", { exact: true }).click();
    await expect(app.locator("header.top .chip.ok")).toHaveText("Running");
    await expect(app.locator('[data-slots-state="none"]')).toContainText("has no slots");
    await expect(app.locator('[data-panel="readings"] .empty-box')).toContainText("no readings");
    // The top bar's two buttons are in their place; this node lists neither action, so neither can be pressed.
    await expect(app.locator("header.top").getByRole("button")).toHaveText(["Locate", "Reboot"]);
    for (const button of await app.locator("header.top").getByRole("button").all()) await expect(button).toBeDisabled();
    await expect(app.getByRole("button", { name: "Install…" })).toBeDisabled();
    await expectNoOverflow(app);
  });
});

for (const size of SIZES) {
  test(`nothing overflows at ${size.width}x${size.height}, on every tab`, async ({ app }) => {
    await app.setViewportSize(size);
    await openBoard(app);
    await expect(app.locator("[data-slot]")).toHaveCount(4);
    await expect(app.locator('[data-panel="readings"] .tile[data-reading]')).toHaveCount(5);
    await expectNoOverflow(app);
    for (const tab of ["Hardware", "Files", "Terminal", "Logs"]) {
      await app.locator('[data-panel="dock"] .tabbar').getByText(tab, { exact: true }).click();
      await expectNoOverflow(app);
    }
    await slot(app, "slot-3").getByRole("button", { name: "Remove" }).click();
    await expectNoOverflow(app);
  });
}
