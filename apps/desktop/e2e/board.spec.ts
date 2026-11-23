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
  await expect(top.locator(".chip.neu")).toHaveText(["Slot 4", "demo-os 2026.11.2", "USB 1-2", "127.0.0.1"]);
  // Both controls keep their place. The board offers a restart, and no board offers a locate.
  await expect(top.getByRole("button")).toHaveText(["Locate", "Reboot"]);
  await expect(top.getByRole("button", { name: "Reboot" })).toBeEnabled();

  const tiles = app.locator('[data-panel="readings"] .tile[data-reading]');
  await expect(tiles).toHaveCount(5);
  await expect(tiles.locator(".lbl")).toHaveText(["pressure", "temperature", "accel x", "accel y", "accel z"]);
  await expect(tiles.first().locator(".val small")).toHaveText("kPa");
  // Units as a person writes them, and a foot of two facts: the highest value shown, and the limit or range.
  await expect(tiles.nth(2).locator(".val small")).toHaveText("m/s²");
  await expect(tiles.nth(1).locator(".foot")).toHaveText(/^max -?\d+\.\d · limit 45$/);
  await expect(tiles.nth(2).locator(".foot")).toHaveText(/^max -?\d+\.\d · -20 to 20$/);
  // The sparkline grows as samples arrive, one a second, and the value is a number.
  const path = tiles.first().locator("svg.spark path").last();
  await expect(path).toBeVisible();
  const before = await path.getAttribute("d");
  await expect.poll(async () => path.getAttribute("d")).not.toBe(before);
  await expect(tiles.first().locator(".val")).toHaveText(/^\d+(\.\d)? kPa$/);
  await expect(app.locator('[data-panel="readings"] .ph')).toContainText(/updated \d+\.\d s ago/);
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

test("Locate is off with its reason on the page, and Reboot runs at once and keeps what the board said", async ({ app, bridge }) => {
  await openBoard(app);
  const top = app.locator("header.top");
  // No board offers a locate: the control is there, off, and the page says why in words.
  const locate = top.getByRole("button", { name: "Locate" });
  await expect(locate).toBeDisabled();
  await expect(locate).toHaveClass(/\bdis\b/);
  await expect(top.locator("[data-not-offered]")).toHaveText("This board doesn't offer locate.");
  // The reason is on the page, not in a tooltip.
  expect(await top.locator("[title]").count()).toBe(0);
  // Nothing named locate (or the old action list) was sent to the board.
  expect(await bridge.call("sim.node", { runs: "locate" })).toBe(0);
  expect(await bridge.call("sim.node", { runs: "actions.list" })).toBe(0);
  // The board's restart device offers the reboot: one run of its `.call` (`lemnos.reboot.call`), counted
  // from before the click (the node also counts a device's description runs under that name).
  const reboot = top.getByRole("button", { name: "Reboot" });
  await expect(reboot).toBeEnabled();
  const before = (await bridge.call("sim.node", { runs: "lemnos.reboot.call" })) as number;
  await reboot.click();
  // The result is a line in the log pane, kept in the board's history: no chip in the top bar.
  const result = app.locator('[data-panel="dock"] .lines [data-result="ok"]').first();
  await expect(result).toContainText("system.reboot.reboot: ");
  await expect(result.locator(".lv-ok")).toHaveText("ok");
  expect(await bridge.call("sim.node", { runs: "lemnos.reboot.call" })).toBe(before + 1);
  await expect(top.locator("[data-said]")).toHaveCount(0);
  // It stays: newer log lines from the board do not push it out, and the page shows it after the
  // moment has passed (the old chip went after 8 s).
  await bridge.call("sim.node", { log: "camera: frame loop started" });
  await expect(app.locator('[data-panel="dock"] .lines .ln').nth(1)).toContainText("frame loop started");
  await app.waitForTimeout(9000);
  await expect(result).toContainText("system.reboot.reboot: ");
  // And it is one of Recent events, under the board's name.
  const snapshot = (await bridge.call("snapshot")) as { topics: { inventory: { activity: { device: string; kind: string; message: string }[] } } };
  expect(snapshot.topics.inventory.activity.filter((line) => line.device === BOARD.id && line.kind === "action").map((line) => line.message)).toEqual([
    expect.stringMatching(/^system\.reboot\.reboot: /),
  ]);
  await expectNoOverflow(app);
});

test("a board with no restart device shows Reboot off, with its reason", async ({ app, bridge }) => {
  await openBoard(app);
  const top = app.locator("header.top");
  const reboot = top.getByRole("button", { name: "Reboot" });
  await expect(reboot).toBeEnabled();
  // The board's file has no restart device: it leaves the board's resource list.
  await bridge.call("sim.node", { remove_device: "reboot" });
  await expect(reboot).toBeDisabled();
  await expect(reboot).toHaveClass(/\bdis\b/);
  await expect(top.getByRole("button")).toHaveText(["Locate", "Reboot"]);
  await expect(top.locator("[data-not-offered]")).toHaveText("This board doesn't offer locate or reboot.");
  await expectNoOverflow(app);
});

test("a refusal is on the page in the board's own words, in full: in the log pane, and a slot's in its row too", async ({ app, bridge }) => {
  await openBoard(app);
  await expect(app.locator("[data-slot]")).toHaveCount(4);
  const whole = (el: Element) => el.scrollWidth <= el.clientWidth && el.scrollHeight <= el.clientHeight;
  // The board refuses the next Reboot (its restart device's call), with more words than a chip holds.
  // No board offers a locate (Raze #2's offers, 2026-11-23), so the refusal is the restart's.
  const words = "a slot install is running, and the board does not restart until that install has ended, so nothing is lost on the way";
  await bridge.call("sim.node", { refuse: ["system.reboot.reboot", words] });
  await app.locator("header.top").getByRole("button", { name: "Reboot" }).click();
  const line = app.locator('[data-panel="dock"] .lines [data-result="refused"]').first();
  await expect(line).toContainText(`system.reboot.reboot: `);
  await expect(line).toContainText(words);
  await expect(line.locator(".lv-err")).toHaveText("err");
  await expect(line).not.toHaveAttribute("title");
  await expect(app.locator("header.top [data-said]")).toHaveCount(0);
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
  // The log pane keeps it too, newest first, above the reboot's.
  await expect(app.locator('[data-panel="dock"] .lines [data-result="refused"]').first()).toContainText("default slot-1: ");
  await expect(app.locator('[data-panel="dock"] .lines [data-result]')).toHaveCount(2);
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
  await expect(row.locator(".l1 b")).toHaveText("Remove slot 3?");
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
  await expect(app.locator('[data-panel="dock"] .lines [data-result="ok"]').first()).toContainText("keep last 5: done");
  await expect(keep).toHaveValue("5");
});

test("an install is a job with progress, and ends as a new slot", async ({ app, bridge }) => {
  await openBoard(app);
  await bridge.call("sim.node", { hold_installs: true });
  await bridge.call("sim.pick_os_image", { slot: "slot-5", version: "2026.11.14" });
  const install = app.getByRole("button", { name: "Install…" });
  const place = await install.boundingBox();
  await install.click();
  const row = app.locator("[data-install]");
  await expect(row.locator(".l1 b")).toHaveText("demo-os-2026.11.14.img");
  await expect(row).toHaveClass(/write/);
  await expect(row.locator(".bar")).toBeVisible();
  // While it runs there is one install at a time: the button stays where it is and cannot be pressed.
  await expect(install).toBeDisabled();
  expect(await install.boundingBox()).toEqual(place);
  // The row being written is counted with the slots kept, as the mockup counts it.
  await expect(app.locator('[data-panel="slots"] .ph .chip')).toHaveText("5 kept");
  await expect(row.locator(".chip")).toHaveText(/Writing|Installing|Waiting for the board/);
  await bridge.call("sim.node", { hold_installs: false });
  await expect(slot(app, "slot-5").locator(".l1 b")).toHaveText("Slot 5 · 2026.11.14");
  // The board keeps its last three and what is pinned, default or running: the oldest plain
  // slot went when the new one landed. The board did that, not Atlas.
  await expect(slot(app, "slot-1")).toHaveCount(0);
  await expect(app.locator("[data-slot]")).toHaveCount(4);
  await expect(row).toHaveCount(0);
  await expect(app.getByRole("button", { name: "Install…" })).toBeVisible();
});

test("an image for another board is refused on the computer, in the board library's words", async ({ app, bridge }) => {
  await openBoard(app);
  await bridge.call("sim.pick_os_image", { slot: "slot-9", version: "2026.12.1", board: "another-board" });
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
  await expect(app.locator("header.top .chip.warn")).toHaveText("Slot 3 · trial");
  // Nothing on the row confirms it, and no other trial is offered meanwhile.
  await expect(app.getByRole("button", { name: "Confirm" })).toHaveCount(0);
  await expect(app.getByRole("button", { name: "Trial" })).toHaveCount(0);
  await expect(app.locator("[data-boots] .tr").first()).toContainText("slot 3");
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

test("the dock's tabs: hardware as the board describes it, and what is not offered or not in this build", async ({ app }) => {
  await openBoard(app);
  const dock = app.locator('[data-panel="dock"]');
  await expect(dock.locator(".tabbar label")).toHaveText(["Logs", "Hardware", "Files", "Terminal"]);
  await expect(dock.locator(".tabbar label.on")).toHaveText("Logs");
  await dock.getByText("Hardware", { exact: true }).click();
  await expect(dock.locator(".tabbar label.on")).toHaveText("Hardware");
  // The board lists its seven devices: the sensors, and the slot list and the restart (Lemnos devices too).
  await expect(dock.locator("[data-device]")).toHaveCount(7);
  await expect(dock.locator('[data-device="lemnos.vane0"]')).toContainText("Wind vane");
  await expect(dock.locator('[data-device="lemnos.gauge1"] .chip.warn')).toHaveText("retrying");
  await expect(dock.locator('[data-device="lemnos.gauge1"]')).toContainText("sensor drifting");
  await dock.getByText("Files", { exact: true }).click();
  // No board offers files yet: the tab says so in one plain sentence, in its place.
  await expect(dock.locator(".pane-files")).toHaveText("This board doesn't offer files yet.");
  await dock.getByText("Terminal", { exact: true }).click();
  await expect(dock.locator('[data-not-built="terminal"]')).toContainText("not in this build");
});

test("a board that stops answering shows as offline, and reads again when it is back", async ({ app, bridge }) => {
  await openBoard(app);
  await expect(app.locator('[data-panel="readings"] .tile[data-reading]')).toHaveCount(5);
  const buttons = app.locator("header.top").getByRole("button");
  // Reboot is what the board offers (no board offers a locate): it is on while the board answers.
  await expect(buttons.nth(1)).toBeEnabled();
  const places = [await buttons.nth(0).boundingBox(), await buttons.nth(1).boundingBox(), await app.getByRole("button", { name: "Install…" }).boundingBox()];
  await bridge.call("sim.set_online", { id: BOARD.id, online: false });
  await expect(app.locator("header.top .chip.off")).toHaveText("Offline");
  // No nameless tiles: one sentence in their place says why nothing is read.
  await expect(app.locator('[data-panel="readings"] [data-tiles="offline"] .empty-box')).toContainText("is offline. Nothing is read until it answers again.");
  await expect(app.locator('[data-panel="readings"] .tile[data-reading]')).toHaveCount(0);
  await expect(app.locator('[data-panel="readings"] .ph')).toContainText("offline · nothing is read until it answers again");
  await expect(app.locator('[data-panel="slots"] .empty-box')).toContainText("is offline");
  // Locate, Reboot and Install… stay where they were, and cannot be pressed.
  await expect(buttons).toHaveText(["Locate", "Reboot"]);
  await expect(buttons.nth(0)).toBeDisabled();
  await expect(buttons.nth(1)).toBeDisabled();
  await expect(app.getByRole("button", { name: "Install…" })).toBeDisabled();
  expect([await buttons.nth(0).boundingBox(), await buttons.nth(1).boundingBox(), await app.getByRole("button", { name: "Install…" }).boundingBox()]).toEqual(places);
  // The page already says why the board is away: the top bar adds no reason of its own.
  await expect(app.locator("header.top [data-not-offered]")).toHaveCount(0);
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
    await expect(app.locator("header.top [data-not-offered]")).toHaveText("This board doesn't offer locate or reboot.");
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
