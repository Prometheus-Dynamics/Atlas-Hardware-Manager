// F5, the flash flow: real pointer clicks against the real core. The board in its boot ROM is
// the simulated one behind the real boot ROM driver and the real loader (atlas-rockchip); the
// card is the simulated one behind the real disk driver and the real write pipeline (atlas-disk).
// The harness calls the core only to change the simulated world: to queue the file the person
// will choose, to make the board silent, to hold or pull the card.
import type { Page } from "@playwright/test";
import { DEMO, SIZES, capMs, demoSettled, expect, expectNoOverflow, test, type Bridge } from "./bridge";

const step = (page: Page, key: string) => page.locator(`[data-step="${key}"]`);

async function openFlash(page: Page) {
  await demoSettled(page);
  await page.locator(".tr.dev", { hasText: DEMO.bootRom.name }).getByRole("button", { name: "Flash" }).click();
  await expect(page.locator("header.top h1")).toHaveText("Flash a board");
  await expect(page.locator('[data-panel="board"] .chip.acc')).toHaveText("found in boot ROM");
}

async function chooseImage(page: Page, bridge: Bridge) {
  await bridge.call("sim.pick_image", {});
  await page.getByRole("button", { name: "Choose image…" }).click();
  await expect(page.locator('[data-panel="image"]')).toHaveAttribute("data-image", "ready");
}

test("a board in its boot ROM: choose an image, send the loader, and the page says where it stops", async ({ app, bridge }) => {
  await openFlash(app);
  await expect(app.locator("header.top .chip.acc")).toHaveText("Rockchip maskrom");
  await expect(app.locator("header.top .chip.neu")).toHaveText(DEMO.bootRom.name);
  // Nothing can be sent before an image is chosen: the button is not there.
  await expect(step(app, "image")).toHaveAttribute("data-state", "cur");
  await expect(step(app, "board")).toHaveAttribute("data-state", "done");
  await expect(step(app, "loader")).toHaveAttribute("data-state", "pend");
  await expect(app.getByRole("button", { name: "Send loader" })).toHaveCount(0);

  await chooseImage(app, bridge);
  await expect(step(app, "image")).toHaveAttribute("data-state", "done");
  await expect(step(app, "image").locator(".sd")).toHaveText("demo-os-2026.11.2.img · 1.6 MB");
  const image = app.locator('[data-panel="image"] dl');
  await expect(image).toContainText("example-board");
  // The slot is read out of the image by the same reader an install uses.
  await expect(image.locator("[data-built-for]")).toHaveText("example-board · slots alpha, beta");
  await expect(image).toContainText("1.6 MB file, 1.6 MB written");
  await expect(image).toContainText("not checked (security is off)");
  await expect(step(app, "loader")).toHaveAttribute("data-state", "cur");
  await expect(step(app, "loader").locator(".sd")).toHaveText("ready to send");

  await app.getByRole("button", { name: "Send loader" }).click();
  await expect(step(app, "loader")).toHaveAttribute("data-state", "done");
  await expect(step(app, "loader").locator(".sd")).toContainText("the board answers in loader mode");
  // The flow stops here in this build, and says so: nothing after the loader is drawn as done.
  await expect(app.locator('[data-flash-chip="loaded"]')).toHaveText("stops after the loader");
  for (const later of ["flasher", "write", "boot"]) {
    await expect(step(app, later)).toHaveAttribute("data-state", "pend");
    await expect(step(app, later).locator(".sd")).toHaveText("not in this build");
  }
  await expect(app.locator('[data-note="limit"]')).toContainText("Nothing was written to the board");
  const sent = app.locator('[data-panel="sent"]');
  await expect(sent.locator("dl")).toContainText("3588");
  await expect(sent.locator(".ln").first()).toContainText("the usbplug answers");
  await expect(app.locator('[data-panel="recent"] .tr').first()).toContainText(DEMO.bootRom.name);
  await expect(app.locator('[data-panel="recent"] .chip').first()).toHaveText("loader ran");
  await expect(app.locator('[data-panel="board"] dl')).toContainText("now in loader mode");

  await app.getByRole("button", { name: "Clear" }).click();
  await expect(app.locator('[data-flash-chip="loaded"]')).toHaveCount(0);
  await expect(sent).toContainText("Nothing has been sent yet.");
});

test.describe("a silent board at the app's own waits", () => {
  // The page's "capped at" is JobTiming::default()'s cap, which the bridge's retry scale does not
  // shorten (TODO(user) in flash.rs): at the app's own waits the page and the runs agree.
  test.use({ retryScale: 1 });

  test("a silent board: the loader is retried, stopped by hand, and runs when Retry is pressed", async ({ app, bridge }) => {
    await openFlash(app);
    await chooseImage(app, bridge);
    await bridge.call("sim.rom", { silent: 400 });
    await app.getByRole("button", { name: "Send loader" }).click();

    const failure = app.locator('[data-panel="failure"]');
    await expect(failure).toHaveAttribute("data-status", "retrying");
    await expect(failure.locator(".ph h2")).toHaveText("Loader failed");
    await expect(failure.locator(".ph .chip.err")).toHaveText("Retrying");
    await expect(app.locator('[data-flash-chip="failed"]')).toHaveText("failed at loader");
    await expect(step(app, "loader")).toHaveAttribute("data-state", "fail");
    await expect(failure.locator("dl")).toContainText("never gives up on its own");
    await expect(failure.locator("dl")).toContainText(`capped at ${capMs(1) / 1000} s between tries`);
    await expect(failure.locator("dl")).toContainText("did not answer");
    // It tries again by itself.
    await expect(failure.locator("dl")).toContainText(/[3-9] so far/, { timeout: 15_000 });
    await expect(app.locator('[data-panel="log"] .ln .lv-err').first()).toBeVisible();

    // The manual off, without moving the panel.
    // The panel is on the page between tries, not during one: its box is read while it is there.
    // The wait in progress is the longest a run still retrying would go without a try.
    const waiting = (await flashRun(bridge)).retry_in_ms;
    expect(waiting, "a wait between tries is in progress or just ended").toBeGreaterThan(0);
    let before: Awaited<ReturnType<typeof failure.boundingBox>> = null;
    await expect.poll(async () => (before = await failure.boundingBox())).not.toBeNull();
    await failure.getByRole("button", { name: "Stop retrying" }).click();
    await expect(failure).toHaveAttribute("data-status", "stopped");
    await expect(failure.locator(".ph .chip")).toHaveText("Stopped");
    // Stopped is its own state: its title, its note and its hint say so, and nothing says "failed".
    await expect(failure.locator(".ph h2")).toHaveText("Loader stopped");
    await expect(failure.locator(".note")).toContainText("Stopped by hand.");
    await expect(failure.locator("[data-bundled]")).toHaveText("Nothing starts again until you press Retry.");
    await expect(app.locator('[data-flash-chip="failed"]')).toHaveText("stopped at loader");
    await expect(failure.locator("dl")).toContainText("not until you press Retry");
    expect(await failure.boundingBox()).toEqual(before);
    const attempts = await failure.locator("dl dd").first().textContent();
    // Nothing happening has no event to wait for: twice the wait that was in progress, in which a
    // run still retrying would have tried at least once.
    await app.waitForTimeout(2 * waiting);
    expect(await failure.locator("dl dd").first().textContent(), "nothing is tried while stopped").toBe(attempts);

    // A board that answers is plugged in; Retry runs the loader at once.
    await bridge.call("sim.rom", { fresh: true });
    await failure.getByRole("button", { name: "Retry", exact: true }).click();
    await expect(step(app, "loader")).toHaveAttribute("data-state", "done");
    await expect(app.locator('[data-panel="recent"] .chip').first()).toHaveText("retried");
    await expect(app.locator('[data-panel="recent"] .chip.err').first()).toHaveText("retried");
  });
});

test("a failing flash says its last error and where its support bundle is, in full", async ({ app, bridge }) => {
  await openFlash(app);
  await chooseImage(app, bridge);
  await bridge.call("sim.rom", { silent: 400 });
  await app.getByRole("button", { name: "Send loader" }).click();
  const failure = app.locator('[data-panel="failure"]');
  await expect(failure.locator(".ph .chip.err")).toHaveText("Retrying");
  const error = failure.locator("[data-last-error]");
  await expect(error).not.toBeEmpty();
  await expect(error).not.toHaveAttribute("title");
  expect(await error.evaluate((el) => el.scrollWidth <= el.clientWidth)).toBe(true);
  await failure.getByRole("button", { name: "Make support bundle" }).click();
  const bundle = failure.locator("[data-bundled]");
  await expect(bundle).toHaveText(/^Support bundle written to \/.+atlas-support-\d+\.txt$/);
  await expect(bundle).not.toHaveAttribute("title");
  expect(await bundle.evaluate((el) => el.scrollWidth <= el.clientWidth)).toBe(true);
  await expectNoOverflow(app);
  await app.getByRole("button", { name: "Cancel" }).click();
});

type FlashRun = { status: string; retry_in_ms: number; retry_at_ms: number; ended_ms: number | null };
const flashRun = async (bridge: Bridge) => ((await bridge.call("snapshot")) as { topics: { flash: { run: FlashRun } } }).topics.flash.run;

test.describe("with the app's own waits between tries", () => {
  // Retry now must be pressed well inside a wait: the app's own seconds, not the bridge's tenths.
  test.use({ retryScale: 1 });

  test("Retry now cuts the wait short", async ({ app, bridge }) => {
    await openFlash(app);
    await chooseImage(app, bridge);
    await bridge.call("sim.rom", { silent: 400 });
    await app.getByRole("button", { name: "Send loader" }).click();
    const failure = app.locator('[data-panel="failure"]');
    // Far enough in for the wait between tries to be seconds long (the fourth try's is 2 s).
    await expect(failure.locator("dl")).toContainText(/([4-9]|\d{2,}) so far/, { timeout: 15_000 });
    let waiting: FlashRun | undefined;
    await expect.poll(async () => {
      waiting = await flashRun(bridge);
      return waiting.status === "retrying" && waiting.retry_at_ms - Date.now() > 1000;
    }).toBe(true);
    await bridge.call("sim.rom", { fresh: true });
    await failure.getByRole("button", { name: "Retry now" }).click();
    await expect(step(app, "loader")).toHaveAttribute("data-state", "done");
    // The press started the try, not the end of the wait: the run ended before its wait was over.
    expect((await flashRun(bridge)).ended_ms).toBeLessThan(waiting!.retry_at_ms);
    // A run that ended can be sent again.
    await expect(app.getByRole("button", { name: "Send loader again" })).toBeVisible();
  });
});

test("Cancel ends a flash that is failing", async ({ app, bridge }) => {
  await openFlash(app);
  await chooseImage(app, bridge);
  await bridge.call("sim.rom", { silent: 400 });
  await app.getByRole("button", { name: "Send loader" }).click();
  const failure = app.locator('[data-panel="failure"]');
  await expect(failure).toHaveAttribute("data-status", "retrying");
  await app.getByRole("button", { name: "Cancel" }).click();
  await expect(failure).toHaveCount(0);
  await expect(step(app, "loader").locator(".sd")).toHaveText("cancelled, nothing was written");
  await expect(app.locator('[data-panel="recent"] .chip').first()).toHaveText("cancelled");
});

test("a file that is not an image is refused in the reader's words, and choosing nothing changes nothing", async ({ app, bridge }) => {
  await openFlash(app);
  const image = app.locator('[data-panel="image"]');
  const panels = [await image.boundingBox(), await app.locator('[data-panel="board"]').boundingBox(), await app.locator('[data-panel="sent"]').boundingBox()];
  await bridge.call("sim.pick_image", { broken: true, name: "notes.img" });
  await app.getByRole("button", { name: "Choose image…" }).click();
  await expect(image).toHaveAttribute("data-image", "refused");
  await expect(image.locator(".note.err")).not.toBeEmpty();
  // The reason stands inside the panel's own rows, in full: no panel has moved or changed size.
  expect([await image.boundingBox(), await app.locator('[data-panel="board"]').boundingBox(), await app.locator('[data-panel="sent"]').boundingBox()]).toEqual(panels);
  const reason = image.locator("[data-refused]");
  await expect(reason).not.toHaveAttribute("title");
  expect(await reason.evaluate((el) => el.scrollWidth <= el.clientWidth && el.scrollHeight <= el.clientHeight)).toBe(true);
  // The file is said with the folder it is in, on the page.
  await expect(image.locator("[data-file]")).toHaveText(/^notes\.img in \/.+sim-files$/);
  await expect(image.locator("[data-file]")).not.toHaveAttribute("title");
  await expect(step(app, "image")).toHaveAttribute("data-state", "fail");
  await expect(app.getByRole("button", { name: "Send loader" })).toHaveCount(0);
  // The dialog closed with nothing chosen.
  await image.getByRole("button", { name: "Choose another…" }).click();
  await expect(image).toHaveAttribute("data-image", "refused");
  // A real image replaces it.
  await bridge.call("sim.pick_image", {});
  await image.getByRole("button", { name: "Choose another…" }).click();
  await expect(image).toHaveAttribute("data-image", "ready");
  await expect(app.getByRole("button", { name: "Send loader" })).toBeVisible();
});

test("the board is unplugged and comes back: the page follows it", async ({ app, bridge }) => {
  await openFlash(app);
  const panels = async () => [await app.locator('[data-panel="board"]').boundingBox(), await app.locator('[data-panel="sent"]').boundingBox()];
  const found = await panels();
  await bridge.call("sim.set_online", { id: DEMO.bootRom.id, online: false });
  await expect(app.locator('[data-board="none"]')).toBeVisible();
  // With no board the panel keeps its size, so the log below it does not move.
  expect(await panels()).toEqual(found);
  await expect(app.locator('[data-panel="board"] .chip.info')).toHaveText("Looking on USB");
  await expect(step(app, "board")).not.toHaveAttribute("data-state", "done");
  await bridge.call("sim.set_online", { id: DEMO.bootRom.id, online: true });
  await expect(app.locator('[data-panel="board"] .chip.acc')).toHaveText("found in boot ROM");
});

test.describe("with nothing plugged in", () => {
  test.use({ scene: "empty" });
  test("the flash screen waits for a board and the disk screen for a disk", async ({ app }) => {
    await app.locator("aside.side").getByText("Flash a board").click();
    await expect(app.locator('[data-board="none"]')).toContainText("No board is in its boot ROM");
    await expect(step(app, "image").locator(".sd")).toHaveText("no image chosen yet");
    await expect(app.getByRole("button", { name: "Send loader" })).toHaveCount(0);
    await expectNoOverflow(app);
    await app.evaluate(() => (location.hash = "#/disk"));
    await expect(app.locator('[data-panel="disks"] .empty-box')).toContainText("No removable disk found");
    await expect(app.locator("[data-foot]")).toContainText("Choose a first-flash image to write.");
    await expectNoOverflow(app);
  });
});

async function openDisk(page: Page) {
  await demoSettled(page);
  await page.locator("aside.side").getByText(DEMO.disk.name, { exact: true }).click();
  await expect(page.locator("header.top h1")).toHaveText("Write a disk");
  await expect(page.locator(`[data-disk="${DEMO.disk.id}"]`)).toHaveClass(/sel/);
}

test("a card: the erase is confirmed in place, then written and read back", async ({ app, bridge }) => {
  await openDisk(app);
  await expect(app.locator("header.top .chip.info")).toHaveText("sdb selected");
  const foot = app.locator("[data-foot]");
  await expect(foot).toContainText("Choose a first-flash image to write.");
  await chooseImage(app, bridge);
  await expect(foot).toContainText("Everything on it is erased.");

  // The question takes the footer's place: nothing above it moves, and the footer keeps its size.
  const before = await foot.boundingBox();
  const table = await app.locator(`[data-disk="${DEMO.disk.id}"]`).boundingBox();
  await foot.getByRole("button", { name: "Erase and write…" }).click();
  await expect(foot).toHaveAttribute("data-foot", "confirm");
  await expect(foot.locator(".q")).toHaveText("Erase sdb (SD card 64 GB) and write demo-os-2026.11.2.img? This cannot be undone.");
  expect(await foot.boundingBox()).toEqual(before);
  expect(await app.locator(`[data-disk="${DEMO.disk.id}"]`).boundingBox()).toEqual(table);
  await foot.getByRole("button", { name: "Cancel" }).click();
  await expect(foot).toHaveAttribute("data-foot", "idle");

  await foot.getByRole("button", { name: "Erase and write…" }).click();
  await foot.getByRole("button", { name: "Erase and write", exact: true }).click();
  await expect(foot).toHaveAttribute("data-foot", "done");
  await expect(foot.locator(".note.ok")).toContainText("1.6 MB written and read back");
  expect(await foot.boundingBox()).toEqual(before);
  await foot.getByRole("button", { name: "Done" }).click();
  await expect(foot).toHaveAttribute("data-foot", "idle");
});

test("with two disks the person chooses which, and the question names the chosen one", async ({ app, bridge }) => {
  await openDisk(app);
  await bridge.call("sim.disk", { stick: true });
  await expect(app.locator("[data-disk]")).toHaveCount(2);
  const stick = app.locator("[data-disk]", { hasText: "USB stick 16 GB" });
  await expect(stick).not.toHaveClass(/sel/);
  await stick.click();
  await expect(stick).toHaveClass(/sel/);
  await expect(app.locator("header.top .chip.info")).toHaveText("sdc selected");
  await chooseImage(app, bridge);
  await app.locator("[data-foot]").getByRole("button", { name: "Erase and write…" }).click();
  await expect(app.locator("[data-foot] .q")).toContainText("Erase sdc (USB stick 16 GB)");
  // Choosing the other disk withdraws the question: it was about the first one.
  await app.locator(`[data-disk="${DEMO.disk.id}"]`).click();
  await expect(app.locator("[data-foot]")).toHaveAttribute("data-foot", "idle");
  await expect(app.locator("[data-foot]")).toContainText("SD card 64 GB");
});

test("a card pulled while it is written is waited for, and a write can be cancelled", async ({ app, bridge }) => {
  await openDisk(app);
  await chooseImage(app, bridge);
  const foot = app.locator("[data-foot]");
  const choose = app.getByRole("button", { name: "Choose another…" });
  const place = await choose.boundingBox();
  await bridge.call("sim.disk", { hold: true });
  await foot.getByRole("button", { name: "Erase and write…" }).click();
  await foot.getByRole("button", { name: "Erase and write", exact: true }).click();
  await expect(foot).toHaveAttribute("data-foot", "writing");
  // While it writes, the button stays where it is and cannot be pressed.
  await expect(choose).toBeDisabled();
  expect(await choose.boundingBox()).toEqual(place);

  await bridge.call("sim.disk", { wait_held: true }); // pulled mid-write, not before the card was opened
  await bridge.call("sim.set_online", { id: DEMO.disk.id, online: false });
  await expect(foot).toContainText("went away", { timeout: 15_000 });
  await bridge.call("sim.disk", { hold: false });
  await bridge.call("sim.set_online", { id: DEMO.disk.id, online: true });
  await expect(foot).toHaveAttribute("data-foot", "done", { timeout: 15_000 });
  await foot.getByRole("button", { name: "Done" }).click();

  // Cancelled part way: the card is said to need a new write.
  await expect(app.locator(`[data-disk="${DEMO.disk.id}"]`)).toHaveClass(/sel/);
  await bridge.call("sim.disk", { hold: true });
  await foot.getByRole("button", { name: "Erase and write…" }).click();
  await foot.getByRole("button", { name: "Erase and write", exact: true }).click();
  await expect(foot).toHaveAttribute("data-foot", "writing");
  await foot.getByRole("button", { name: "Cancel" }).click();
  await bridge.call("sim.disk", { hold: false });
  await expect(foot).toHaveAttribute("data-foot", "cancelled");
  await expect(foot.locator(".note.err")).toContainText("partly written and needs a new write");
});

for (const size of SIZES) {
  test(`the flash screens do not overflow at ${size.width}x${size.height}`, async ({ app, bridge }) => {
    await app.setViewportSize(size);
    await openFlash(app);
    await expectNoOverflow(app);
    await chooseImage(app, bridge);
    await bridge.call("sim.rom", { silent: 400 });
    await app.getByRole("button", { name: "Send loader" }).click();
    await expect(app.locator('[data-panel="failure"]')).toBeVisible();
    await expect(app.locator('[data-panel="log"] .ln').nth(2)).toBeVisible();
    await expectNoOverflow(app);
    // Stopped by hand before the board answers: while it is retrying, the next try (a quarter of a
    // second in, at first) sends the loader on its own once the board answers, and the panel with
    // its buttons goes away before a click can land. Stopped, nothing runs until Retry is pressed.
    const failure = app.locator('[data-panel="failure"]');
    await failure.getByRole("button", { name: "Stop retrying" }).click();
    await expect(failure).toHaveAttribute("data-status", "stopped");
    await expectNoOverflow(app);
    await bridge.call("sim.rom", { fresh: true });
    await failure.getByRole("button", { name: "Retry", exact: true }).click();
    await expect(app.locator('[data-flash-chip="loaded"]')).toBeVisible();
    await expectNoOverflow(app);
    await app.locator("aside.side").getByText(DEMO.disk.name, { exact: true }).click();
    await app.locator("[data-foot]").getByRole("button", { name: "Erase and write…" }).click();
    await expect(app.locator("[data-foot] .q")).toBeVisible();
    await expectNoOverflow(app);
  });
}
