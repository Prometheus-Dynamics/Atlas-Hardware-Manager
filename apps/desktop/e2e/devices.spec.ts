// F3, the device list: every record in every state in one list, with what Atlas can do for
// each. Real pointer clicks against the real core; the devices are the simulated ones, each
// behind the driver that finds its real counterpart (see src-tauri/src/sim.rs).
import type { Locator, Page } from "@playwright/test";
import { DEMO, SIZES, demoSettled, expect, expectNoOverflow, holdSnapshot, test } from "./bridge";

const COLUMNS = ["Board", "State", "Reached by", "Default", "Seen"];
const row = (page: Page, record: { id: string }): Locator => page.locator(`.tr.dev[data-device="${record.id}"]`);
const cells = (r: Locator) => r.locator(":scope > div");
const title = (page: Page) => page.locator("header.top h1");

async function listSettled(page: Page) {
  await demoSettled(page);
  await expect(page.locator(".tr.dev")).toHaveCount(7);
}

test("every state is one row, under its group, with its words and its links", async ({ app }) => {
  await listSettled(app);
  await expect(title(app)).toHaveText("Devices");
  await expect(app.locator(".tr.h > div")).toHaveText(COLUMNS);
  await expect(app.locator(".ph .chip.neu")).toHaveText("7 records");
  await expect(app.locator(".ph .chip.info")).toHaveText("Scanning USB and network");
  await expect(app.locator(".tr.g")).toHaveText([
    "Practice field · robot (its devices are read-only)",
    "Boards on this computer",
    "Removable disks",
    "Offline",
  ]);
  // Rows in the order of the list: the robot's controller, the boards by name, the disk, the offline board.
  await expect(app.locator(".tr.dev b")).toHaveText(["sc-practice", "field-radio", "bench-a", "bench-b", "bench-c", "SD card 64 GB", "old-bench"]);

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
  // Beside its name: its chip family in two words. The USB id it was found by is not here.
  await expect(cells(rom).nth(0)).toHaveText("bench-b Rockchip family");
  await expect(app.locator(".tr.dev button")).toHaveCount(1);

  await expect(row(app, DEMO.recovery).locator(".chip.warn")).toHaveText("Recovery");
  await expect(cells(row(app, DEMO.recovery)).nth(2).locator(".chip.neu")).toHaveText(["USB 1-6"]);

  const disk = row(app, DEMO.disk);
  await expect(cells(disk).nth(0)).toHaveText("SD card 64 GB sdb");
  await expect(disk.locator(".chip.info").first()).toHaveText("Removable disk");
  await expect(cells(disk).nth(2).locator(".chip.neu")).toHaveText(["card reader"]);

  // The offline board: dimmed, the dashed chip, its last link as plain text, and when it was last seen.
  const offline = row(app, DEMO.offline);
  await expect(offline).toHaveClass(/dim/);
  await expect(offline.locator(".chip.off")).toHaveText("Offline");
  await expect(cells(offline).nth(2)).toHaveText("last USB 1-4 · 10.42.1.9");
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
    [DEMO.offline, /#\/board\/sim-bench%3ASIM-OLD-1$/],
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
  await expect(app.locator("aside.side a.nav.on")).toHaveText("Flash a board");
});

test("a board that is unplugged moves to Offline in place, says so in the events, and comes back", async ({ app, bridge }) => {
  await listSettled(app);
  const board = row(app, DEMO.running);
  await bridge.call("sim.set_online", { id: DEMO.running.id, online: false });
  await expect(board.locator(".chip.off")).toHaveText("Offline");
  await expect(cells(board).nth(2)).toHaveText(/^last /);
  await expect(app.locator(".tr.dev b")).toHaveText(["sc-practice", "field-radio", "bench-b", "bench-c", "SD card 64 GB", "bench-a", "old-bench"]);
  const first = app.locator(".lines .ln").first();
  await expect(first.locator("span").nth(1)).toHaveText("warn");
  await expect(first.locator("span").nth(2)).toHaveText("bench-a: not seen on any link");
  await expect(app.locator(".ph .chip.neu")).toHaveText("7 records");

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
  const chip = app.locator(".ph [data-scan]");
  const button = app.locator(".ph button");
  await expect(button).toHaveText("Pause");
  // The chip ends the header, flush right, as the mockup draws it; the button stands before it.
  await expect(app.locator(".ph").first().locator("> :last-child")).toHaveAttribute("data-scan", "on");
  const edge = await app.locator(".ph").first().evaluate((el) => el.getBoundingClientRect().right - parseFloat(getComputedStyle(el).paddingRight));
  expect(await chip.evaluate((el) => el.getBoundingClientRect().right)).toBe(edge);
  await expect(chip.locator(".dot")).toHaveClass(/\bspin\b/);
  const before = { button: await button.boundingBox(), row: await row(app, DEMO.running).boundingBox(), right: await chip.evaluate((el) => el.getBoundingClientRect().right) };
  await button.click();
  await expect(chip).toHaveText("Scanning is off");
  await expect(chip).toHaveClass(/\bchip off\b/);
  await expect(chip.locator(".dot")).toHaveClass(/\bdot off\b/);
  await expect(button).toHaveText("Resume");
  expect(await button.boundingBox()).toEqual(before.button);
  expect(await row(app, DEMO.running).boundingBox()).toEqual(before.row);
  expect(await chip.evaluate((el) => el.getBoundingClientRect().right)).toBe(before.right);

  // While it is off a board that is unplugged stays as it was last known.
  await bridge.call("sim.set_online", { id: DEMO.recovery.id, online: false });
  await app.waitForTimeout(1000); // four scan rounds of the UI bridge (250 ms each)
  await expect(row(app, DEMO.recovery).locator(".chip.warn")).toHaveText("Recovery");
  await button.click();
  await expect(chip).toHaveText("Scanning USB and network");
  await expect(row(app, DEMO.recovery).locator(".chip.off")).toHaveText("Offline");
});

test("the events leave out a rename to the same name and say found and followed in one line", async ({ app, bridge }) => {
  await listSettled(app);
  // The core's history does hold such lines: every simulated board was named as a person would.
  const snapshot = (await bridge.call("snapshot")) as { topics: { inventory: { activity: { name: string; message: string }[] } } };
  const history = snapshot.topics.inventory.activity;
  expect(history.filter((line) => line.message === `renamed to ${line.name}`).length).toBeGreaterThan(0);
  const lines = await app.locator(".lines .ln > span:nth-child(3)").allTextContents();
  expect(lines.length).toBeGreaterThan(0);
  expect(lines.filter((text) => /^(.+): renamed to \1$/.test(text))).toEqual([]);
  // "now followed over Y" never stands alone straight after "found on X" for the same board.
  const alone = lines.filter((text, index) => {
    const name = `${text.split(":")[0]}:`;
    return text.startsWith(`${name} now followed over `) && lines.slice(index + 1).find((older) => older.startsWith(name))?.startsWith(`${name} found on `);
  });
  expect(alone).toEqual([]);
  // A link is called by its label in a line, never by its id.
  expect(lines.filter((text) => /\b(usb-1-\d|lan|disks)\b/.test(text))).toEqual([]);
});

test("a link that is not answering is a row that says why, in words", async ({ app, bridge }) => {
  await listSettled(app);
  await bridge.call("sim.disk", { broken: true });
  const chip = app.locator(".ph [data-failing]");
  await expect(chip).toHaveText("1 link not answering");
  await expect(chip).not.toHaveAttribute("title");
  const link = app.locator('.tr[data-link="disks"]');
  await expect(link).toHaveClass(/\bwarnrow\b/);
  await expect(cells(link).nth(0)).toHaveText("card reader link");
  await expect(link.locator(".chip.warn")).toHaveText("Not answering");
  await expect(cells(link).nth(2)).toHaveText(/^device did not respond: could not list disks: the card reader did not answer\. Try \d+ in \d+ s\.$/);
  // The reason is read without hovering: nothing of it is cut off.
  expect(await cells(link).nth(2).evaluate((el) => el.scrollWidth <= el.clientWidth && el.scrollHeight <= el.clientHeight)).toBe(true);
  await expect(app.locator(".tr.g").first()).toContainText("Not answering");
  await expectNoOverflow(app);
  await bridge.call("sim.disk", { broken: false });
  await expect(link).toHaveCount(0);
  await expect(chip).toHaveCount(0);
});

test("the list waits for the core's first answer as bars, and says nothing about what is there", async ({ page, bridge }) => {
  const release = await holdSnapshot(page);
  await page.goto(bridge.url);
  await expect(page.locator(".tr.dev[data-loading] .ph-skel").first()).toBeVisible();
  await expect(page.locator(".tr.h > div")).toHaveText(COLUMNS);
  await expect(page.locator(".empty-box")).toHaveCount(0);
  await expect(page.locator(".main")).not.toContainText("Nothing found yet");
  await expect(page.locator(".main")).not.toContainText("records");
  release();
  await listSettled(page);
  await expect(page.locator("[data-loading]")).toHaveCount(0);
});

test.describe("with nothing plugged in", () => {
  test.use({ scene: "empty" });

  test("scanning switched off says so in the header and in the box, and a failing link says why", async ({ app, bridge }) => {
    await expect(app.locator(".empty-box")).toContainText("Nothing found yet");
    await app.locator(".ph").getByRole("button", { name: "Pause" }).click();
    const chip = app.locator(".ph [data-scan]");
    await expect(chip).toHaveText("Scanning is off");
    await expect(chip).toHaveClass(/\bchip off\b/);
    await expect(app.locator(".ph > :last-child")).toHaveAttribute("data-scan", "off");
    await expect(app.locator(".empty-box")).toContainText("Scanning is off, so nothing new is found.");
    await app.locator(".ph").getByRole("button", { name: "Resume" }).click();
    await expect(chip).toHaveText("Scanning USB and network");
    await bridge.call("sim.disk", { broken: true });
    await expect(app.locator('.empty-box .note.warn[data-link="disks"]')).toHaveText(/^card reader is not answering: device did not respond: could not list disks: the card reader did not answer\. Try \d+ in \d+ s\.$/);
    await expectNoOverflow(app);
  });

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
