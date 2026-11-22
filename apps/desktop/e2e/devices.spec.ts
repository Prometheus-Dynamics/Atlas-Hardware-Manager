// F3, the device list: every record in every state in one list, with what Atlas can do for
// each. Real pointer clicks against the real core; the devices are the simulated ones, each
// behind the driver that finds its real counterpart (see src-tauri/src/sim.rs).
import type { Locator, Page } from "@playwright/test";
import { DEMO, SIZES, demoSettled, expect, expectNoOverflow, test } from "./bridge";

const COLUMNS = ["Board", "State", "Reached by", "Default", "Seen"];
const row = (page: Page, record: { id: string }): Locator => page.locator(`.tr.dev[data-device="${record.id}"]`);
const cells = (r: Locator) => r.locator(":scope > div");
const title = (page: Page) => page.locator("header.top h1");

async function listSettled(page: Page) {
  await demoSettled(page);
  await expect(page.locator(".tr.dev")).toHaveCount(6);
}

test("every state is one row, under its group, with its words and its links", async ({ app }) => {
  await listSettled(app);
  await expect(title(app)).toHaveText("Devices");
  await expect(app.locator(".tr.h > div")).toHaveText(COLUMNS);
  await expect(app.locator(".ph .chip.neu")).toHaveText("6 records");
  await expect(app.locator(".ph .chip.info")).toHaveText("Scanning USB and network");
  await expect(app.locator(".tr.g")).toHaveText([
    "Practice field · robot (its devices are read-only)",
    "Boards on this computer",
    "Removable disks",
    "Offline",
  ]);
  // Rows in the order of the list: the robot's controller, the boards by name, the disk, the offline board.
  await expect(app.locator(".tr.dev b")).toHaveText(["sc-practice", "bench-a", "bench-b", "bench-c", "SD card 64 GB", "old-bench"]);

  // The running board: one record with both of its links as chips.
  const running = row(app, DEMO.running);
  await expect(cells(running).nth(0)).toHaveText("bench-a demo-os 2026.11.2");
  await expect(running.locator(".chip.ok")).toHaveText("Running");
  await expect(cells(running).nth(2).locator(".chip.neu")).toHaveText(["USB 1-2", "127.0.0.1"]);
  await expect(cells(running).nth(4)).toHaveText("now");
  await expect(running.locator("button")).toHaveCount(0);

  // The controller that does not run Orion yet.
  const controller = row(app, DEMO.controller);
  await expect(cells(controller).nth(0)).toHaveText("sc-practice SystemCore");
  await expect(controller.locator(".chip.warn")).toHaveText("Waiting for Orion");
  await expect(cells(controller).nth(2).locator(".chip.neu")).toHaveText(["network 10.99.99.2"]);

  // The board in its boot ROM is the only row that offers Flash.
  const rom = row(app, DEMO.bootRom);
  await expect(rom.locator(".chip.acc")).toHaveText("Boot ROM, waiting");
  await expect(rom.locator("button.btn.sm.pri")).toHaveText("Flash");
  await expect(cells(rom).nth(2).locator(".chip.neu")).toHaveText(["USB 1-3 · boot ROM"]);
  await expect(app.locator(".tr.dev button")).toHaveCount(1);

  await expect(row(app, DEMO.recovery).locator(".chip.warn")).toHaveText("Recovery");

  const disk = row(app, DEMO.disk);
  await expect(cells(disk).nth(0)).toHaveText("SD card 64 GB sim-card");
  await expect(disk.locator(".chip.info").first()).toHaveText("Removable disk");
  await expect(cells(disk).nth(2).locator(".chip.neu")).toHaveText(["card reader"]);

  // The offline board: dimmed, the dashed chip, its last link as plain text, and when it was last seen.
  const offline = row(app, DEMO.offline);
  await expect(offline).toHaveClass(/dim/);
  await expect(offline.locator(".chip.off")).toHaveText("Offline");
  await expect(cells(offline).nth(2)).toHaveText("last simulated");
  await expect(cells(offline).nth(2).locator(".chip")).toHaveCount(0);
  await expect(cells(offline).nth(4)).toHaveText(/^\d\d:\d\d$/);
  await expect(cells(offline).nth(0)).toHaveText("old-bench demo-os 2026.10.20");
  await expect(cells(row(app, DEMO.recovery)).nth(0)).toHaveText("bench-c simulated-recovery 2.1.0");

  // Each row's dot is the tree's dot for the same record.
  for (const [record, tone] of [[DEMO.running, "ok"], [DEMO.bootRom, "acc"], [DEMO.recovery, "warn"], [DEMO.controller, "warn"], [DEMO.disk, "info"], [DEMO.offline, "off"]] as const) {
    await expect(row(app, record).locator(".chip .dot").first()).toHaveClass(new RegExp(`\\b${tone}\\b`));
    await expect(app.locator("aside.side a.nav", { hasText: record.name }).locator(".dot")).toHaveClass(new RegExp(`\\b${tone}\\b`));
  }
});

test("a row opens its record's screen, and Flash opens the flash flow for that board", async ({ app }) => {
  await listSettled(app);
  const opens: [{ id: string }, RegExp][] = [
    [DEMO.running, /#\/board\/simulated%3A0123456789abcdef$/],
    [DEMO.controller, /#\/systemcore\/systemcore%3A9999$/],
    [DEMO.disk, /#\/disk\/removable-disk%3ASIM-CARD-1$/],
    [DEMO.bootRom, /#\/flash\/rockchip-maskrom%3Ausb-1-3$/],
    [DEMO.offline, /#\/board\/simulated%3ASIM-OLD-1$/],
  ];
  for (const [record, url] of opens) {
    await cells(row(app, record)).nth(0).click();
    await expect(app).toHaveURL(url);
    await app.goBack();
    await expect(title(app)).toHaveText("Devices");
  }
  await row(app, DEMO.bootRom).getByRole("button", { name: "Flash" }).click();
  await expect(app).toHaveURL(/#\/flash\/rockchip-maskrom%3Ausb-1-3$/);
  await expect(title(app)).toHaveText("Flash a board");
  await expect(app.locator("aside.side a.nav.on")).toHaveText("bench-b");
});

test("a board that is unplugged moves to Offline in place, says so in the events, and comes back", async ({ app, bridge }) => {
  await listSettled(app);
  const board = row(app, DEMO.running);
  await bridge.call("sim.set_online", { id: DEMO.running.id, online: false });
  await expect(board.locator(".chip.off")).toHaveText("Offline");
  await expect(cells(board).nth(2)).toHaveText(/^last /);
  await expect(app.locator(".tr.dev b")).toHaveText(["sc-practice", "bench-b", "bench-c", "SD card 64 GB", "bench-a", "old-bench"]);
  const first = app.locator(".lines .ln").first();
  await expect(first.locator("span").nth(1)).toHaveText("warn");
  await expect(first.locator("span").nth(2)).toHaveText("bench-a: not seen on any link");
  await expect(app.locator(".ph .chip.neu")).toHaveText("6 records");

  await bridge.call("sim.set_online", { id: DEMO.running.id, online: true });
  await expect(board.locator(".chip.ok")).toHaveText("Running");
  await expect(app.locator(".lines")).toContainText("bench-a: back on ");
});

test("the boot ROM row loses Flash when the board is unplugged", async ({ app, bridge }) => {
  await listSettled(app);
  await bridge.call("sim.set_online", { id: DEMO.bootRom.id, online: false });
  await expect(row(app, DEMO.bootRom).locator(".chip.off")).toHaveText("Offline");
  await expect(app.locator(".tr.dev button")).toHaveCount(0);
  await bridge.call("sim.set_online", { id: DEMO.bootRom.id, online: true });
  await expect(row(app, DEMO.bootRom).getByRole("button", { name: "Flash" })).toBeVisible();
});

test("the scan's manual off is one click, changes no layout, and stops finding", async ({ app, bridge }) => {
  await listSettled(app);
  const chip = app.locator(".ph .chip").last();
  const button = app.locator(".ph button");
  await expect(button).toHaveText("Pause");
  const before = { button: await button.boundingBox(), row: await row(app, DEMO.running).boundingBox(), right: await chip.evaluate((el) => el.getBoundingClientRect().right) };
  await button.click();
  await expect(chip).toHaveText("Scanning is off");
  await expect(chip).toHaveClass(/\boff\b/);
  await expect(button).toHaveText("Resume");
  expect(await button.boundingBox()).toEqual(before.button);
  expect(await row(app, DEMO.running).boundingBox()).toEqual(before.row);
  expect(await chip.evaluate((el) => el.getBoundingClientRect().right)).toBe(before.right);

  // While it is off a board that is unplugged stays as it was last known.
  await bridge.call("sim.set_online", { id: DEMO.recovery.id, online: false });
  await app.waitForTimeout(3000); // longer than a scan round
  await expect(row(app, DEMO.recovery).locator(".chip.warn")).toHaveText("Recovery");
  await button.click();
  await expect(chip).toHaveText("Scanning USB and network");
  await expect(row(app, DEMO.recovery).locator(".chip.off")).toHaveText("Offline");
});

test.describe("with nothing plugged in", () => {
  test.use({ scene: "empty" });

  test("the list says so, keeps scanning, and offers the flash flow", async ({ app }) => {
    await expect(app.locator(".empty-box")).toContainText("Nothing found yet");
    await expect(app.locator(".ph .chip.neu")).toHaveText("0 records");
    await expect(app.locator(".ph .chip.info")).toHaveText("Scanning USB and network");
    await expect(app.locator(".panel")).toHaveCount(1);
    await expect(app.locator("aside.side .sec")).toHaveText(["Devices", "Tools"]);
    await expect(app.locator("aside.side a.nav.off")).toHaveText("No devices yet");
    await expectNoOverflow(app);
    await app.locator(".empty-box").getByText("Flash a board").click();
    await expect(title(app)).toHaveText("Flash a board");
  });
});

for (const size of SIZES) {
  test(`the list fits at ${size.width}x${size.height}: nothing overflows and rows keep their height`, async ({ app }) => {
    await app.setViewportSize(size);
    await listSettled(app);
    await expectNoOverflow(app);
    const heights = await app.locator(".tr.dev").evaluateAll((rows) => rows.map((r) => Math.round(r.getBoundingClientRect().height)));
    // 34 px on a short window, 42 px on a taller one, as the design language sets.
    expect(new Set(heights)).toEqual(new Set([size.height <= 760 ? 34 : 42]));
    expect(await app.locator(".body > .panel").last().evaluate((el) => el.getBoundingClientRect().height)).toBe(150);
    // No cell's text is wider than its cell without being cut by an ellipsis.
    const cut = await app.locator(".tr.dev > div").evaluateAll((all) => all.filter((c) => getComputedStyle(c).textOverflow !== "ellipsis").length);
    expect(cut).toBe(0);
  });
}
