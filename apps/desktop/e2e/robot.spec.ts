// F6, the robot and its controller: both screens against the simulated robot network (a
// controller without Orion and a radio behind the real robot drivers), by real pointer clicks.
import { DEMO, SIZES, demoSettled, expect, expectNoOverflow, test } from "./bridge";

const ROBOT = "Practice field";

test("the tree and the list lead to the robot, and the robot to its controller and back", async ({ app }) => {
  await demoSettled(app);
  const side = app.locator("aside.side");
  await side.getByText(ROBOT, { exact: true }).click();
  await expect(app.locator("header.top h1")).toHaveText(ROBOT);
  await expect(app.locator("header.top .chip")).toHaveText(["robot", "team 9999", "0 boards behind a SystemCore"]);
  await expect(side.locator("a.nav.on")).toHaveText(`${ROBOT}robot`);

  // The robot's own devices, read-only: no button in the table.
  const rows = app.locator("[data-robot-device]");
  await expect(rows).toHaveCount(2);
  await expect(app.locator('[data-panel="robot-devices"] button')).toHaveCount(0);
  const radio = app.locator(`[data-robot-device="${DEMO.radio.id}"]`);
  await expect(radio.locator("b")).toHaveText(DEMO.radio.name);
  await expect(radio.locator(".chip")).toHaveText("reporting");
  await expect(radio).toContainText("robot radio · channel 53 · 6 GHz linked, -58 dBm, good");
  await expect(radio).toContainText("firmware 1.2.0");

  // The controller's summary, and the one primary action.
  const summary = app.locator('[data-panel="controller"]');
  await expect(summary.locator(".chip")).toHaveText("Needs Orion");
  await expect(summary.locator("dd")).toHaveText([DEMO.controller.name, "10.99.99.2", "none seen", "not installed yet"]);
  await summary.getByText("Open SystemCore").click();
  await expect(app.locator("header.top h1")).toHaveText("SystemCore");
  await expect(app.locator("header.top .crumb")).toHaveText(`Devices / ${ROBOT} /`);
  await expect(side.locator("a.nav.on")).toHaveText(DEMO.controller.name);

  // And from the list.
  await side.getByText("All devices").click();
  await app.locator(".tr.dev", { hasText: DEMO.controller.name }).click();
  await expect(app.locator("header.top h1")).toHaveText("SystemCore");
});

test("the controller's screen says what is known, offers nothing it cannot do, and opens its own page", async ({ app, bridge }) => {
  await demoSettled(app);
  await app.locator("aside.side").getByText(DEMO.controller.name, { exact: true }).click();
  await expect(app.locator("header.top .chip")).toHaveText([DEMO.controller.name, "Orion not installed"]);
  const device = app.locator('[data-panel="device"]');
  await expect(device.locator("dd").nth(0)).toHaveText("SystemCore");
  await expect(device.locator("dd").nth(1)).toHaveText("9999");
  await expect(device.locator("dd").nth(3)).toHaveText("10.99.99.2");
  // No install library in this build: the offer says so and shows no button.
  const install = app.locator('[data-panel="install"]');
  await expect(install.locator(".ph .chip")).toHaveText("no package in this build");
  await expect(install.locator("button")).toHaveCount(0);
  await expect(install.locator(".chip.off")).toHaveText(["no package in this build", "not in this build", "after Orion"]);
  // What Atlas saw of it.
  await expect(app.locator('[data-panel="log"] .ln').first()).toBeVisible();

  await app.getByRole("button", { name: "Open its own page" }).click();
  await expect.poll(() => bridge.call("sim.opened")).toEqual(["http://10.99.99.2/"]);
});

// The robot drivers keep a connect's answer for twenty seconds (atlas_robot::CACHE_FOR), so a
// device that left or came back is seen within that, not at once.
const SEEN = { timeout: 40_000 };

test("a robot device that stops answering is not reporting, and reports again when it is back", async ({ app, bridge }) => {
  test.setTimeout(180_000);
  await demoSettled(app);
  await app.locator("aside.side").getByText(ROBOT, { exact: true }).click();
  const radio = app.locator(`[data-robot-device="${DEMO.radio.id}"]`);
  await expect(radio.locator(".chip")).toHaveText("reporting");
  await bridge.call("sim.set_online", { id: DEMO.radio.id, online: false });
  await expect(radio.locator(".chip")).toHaveText("not reporting", SEEN);
  // It stays in its robot: nothing of it is moved to Offline in the tree.
  await expect(app.locator("aside.side .sec")).toHaveText(["Devices", "Offline", "Disks", "Tools"]);
  await expect(app.locator("aside.side a.nav.off")).toHaveText([DEMO.offline.name]);
  await bridge.call("sim.set_online", { id: DEMO.radio.id, online: true });
  await expect(radio.locator(".chip")).toHaveText("reporting", SEEN);

  // The controller gone: its screen says so and offers no page to open.
  await bridge.call("sim.set_online", { id: DEMO.controller.id, online: false });
  await expect(app.locator('[data-panel="controller"] .chip')).toHaveText("Offline", SEEN);
  // In the tree it stays under its robot, drawn as an offline row is: faint, with the grey dot.
  await expect(app.locator("aside.side a.nav.off")).toHaveText([DEMO.controller.name, DEMO.offline.name]);
  await app.locator('[data-panel="controller"]').getByText("Open SystemCore").click();
  await expect(app.locator("header.top .chip")).toHaveText([DEMO.controller.name, "Offline"]);
  await expect(app.locator("header.top").getByRole("button")).toHaveCount(0);
});

test("a robot nobody added and a controller nobody knows say so", async ({ app }) => {
  await demoSettled(app);
  await app.evaluate(() => (location.hash = "#/robot/Nowhere"));
  await expect(app.locator(".empty-box")).toHaveText("There is no robot named Nowhere.");
  await app.evaluate(() => (location.hash = "#/systemcore/systemcore%3A1"));
  await expect(app.locator(".empty-box")).toHaveText("Atlas knows no controller under this name.");
});

for (const size of SIZES) {
  test(`the robot screens do not overflow at ${size.width}x${size.height}`, async ({ app }) => {
    await app.setViewportSize(size);
    await demoSettled(app);
    await app.locator("aside.side").getByText(ROBOT, { exact: true }).click();
    await expect(app.locator("[data-robot-device]")).toHaveCount(2);
    await expectNoOverflow(app);
    await app.locator('[data-panel="controller"]').getByText("Open SystemCore").click();
    await expect(app.locator('[data-panel="install"]')).toBeVisible();
    await expectNoOverflow(app);
  });
}
