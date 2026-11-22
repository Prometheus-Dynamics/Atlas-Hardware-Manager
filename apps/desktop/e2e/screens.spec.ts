// Pictures of each screen and of its mockup, for comparing the two side by side. Not a test of
// anything: it runs only when ATLAS_SCREENS_DIR names where the pictures go
// (scripts/ui-gate.sh --screens DIR). The mockups are drawn with the app's own IBM Plex files,
// which is the font the design language names; without them a mockup falls back to the system
// font and the two pictures would differ in every glyph.
import { mkdirSync } from "node:fs";
import { join, resolve } from "node:path";
import { pathToFileURL } from "node:url";
import type { Page } from "@playwright/test";
import { DEMO, demoSettled, expect, test, type Bridge, type Scene } from "./bridge";

const OUT = process.env.ATLAS_SCREENS_DIR ?? "";
const MOCKUPS = resolve(import.meta.dirname, "../../../docs/mockups");
const FONTS = resolve(import.meta.dirname, "../node_modules/@fontsource");

const font = (family: string, pkg: string, weight: number) =>
  `@font-face{font-family:"${family}";font-weight:${weight};font-style:normal;src:url("${pathToFileURL(
    join(FONTS, pkg, "files", `${pkg}-latin-${weight}-normal.woff2`),
  )}") format("woff2");}`;
const PLEX = [400, 500, 600].map((w) => font("IBM Plex Sans", "ibm-plex-sans", w)).join("") + font("IBM Plex Mono", "ibm-plex-mono", 400);

/** One screen: its name, where it is in the app, what must be on it before the picture, and
 *  the mockup it is built from. Each item adds its screens here. */
export type Screen = { name: string; hash: string; ready: (page: Page, bridge: Bridge) => Promise<void>; mockup?: string; scene?: Scene };

const BOARD = `#/board/${encodeURIComponent(DEMO.running.id)}`;
async function boardRead(page: Page) {
  await demoSettled(page);
  await expect(page.locator('[data-panel="readings"] .tile')).toHaveCount(5);
  await expect(page.locator("[data-slot]")).toHaveCount(4);
}

const FLASH = `#/flash/${encodeURIComponent(DEMO.bootRom.id)}`;
const DISK = `#/disk/${encodeURIComponent(DEMO.disk.id)}`;

async function flashReady(page: Page, bridge: Bridge) {
  await demoSettled(page);
  await bridge.call("sim.pick_image", {});
  await page.getByRole("button", { name: "Choose image…" }).click();
  await expect(page.locator('[data-panel="image"]')).toHaveAttribute("data-image", "ready");
}

async function diskReady(page: Page, bridge: Bridge) {
  await flashReady(page, bridge);
  await expect(page.locator("[data-disk]")).toHaveCount(1);
}

const ROBOT = `#/robot/${encodeURIComponent("Practice field")}`;
const SYSTEMCORE = `#/systemcore/${encodeURIComponent(DEMO.controller.id)}`;

async function settingsOpen(page: Page) {
  await demoSettled(page);
  await expect(page.locator('[data-panel="keys"] [data-key]')).toHaveCount(1);
  await expect(page.locator('[data-panel="support"] dd').first()).toContainText("atlas.log");
}

export const SCREENS: Screen[] = [
  {
    name: "settings",
    hash: "#/settings",
    mockup: "settings.html",
    ready: async (page, bridge) => {
      await settingsOpen(page);
      // A key of the person's own, and one check on record, as the mockup shows a filled page.
      await bridge.call("sim.pick_key", {});
      await page.getByRole("button", { name: "Add key…" }).click();
      await expect(page.locator('[data-panel="keys"] [data-key]')).toHaveCount(2);
      await bridge.call("sim.pick_image", {});
      await page.evaluate(() => (location.hash = "#/flash"));
      await page.getByRole("button", { name: "Choose image…" }).click();
      await expect(page.locator('[data-panel="image"]')).toHaveAttribute("data-image", "ready");
      await page.evaluate(() => (location.hash = "#/settings"));
      await expect(page.locator('[data-panel="checks"] .tr')).toHaveCount(1);
      // A release feed that has nothing newer, checked: the mockup's "checked just now".
      await bridge.call("sim.feed", { state: "nothing", kind: "deb" });
      await page.getByRole("button", { name: "Check for updates" }).click();
      await expect(page.locator("[data-update]")).toHaveText("up to date");
    },
  },
  {
    name: "settings-no-source",
    hash: "#/settings",
    ready: async (page) => {
      await settingsOpen(page);
      await expect(page.locator("[data-update-controls]")).toHaveText("no update source configured");
    },
  },
  {
    name: "settings-update-downloading",
    hash: "#/settings",
    ready: async (page, bridge) => {
      await settingsOpen(page);
      await bridge.call("sim.feed", { state: "newer", kind: "deb", hold: true });
      await page.getByRole("button", { name: "Check for updates" }).click();
      await page.getByRole("button", { name: "Download" }).click();
      await expect(page.locator("[data-update] .bar")).toBeVisible();
      await expect(page.locator("[data-update]")).toContainText("25%");
    },
  },
  {
    name: "settings-update-ready",
    hash: "#/settings",
    ready: async (page, bridge) => {
      await settingsOpen(page);
      await bridge.call("sim.feed", { state: "newer", kind: "deb" });
      await page.getByRole("button", { name: "Check for updates" }).click();
      await page.getByRole("button", { name: "Download" }).click();
      await expect(page.getByRole("button", { name: "Open installer" })).toBeVisible();
    },
  },
  {
    name: "settings-update-refused",
    hash: "#/settings",
    ready: async (page, bridge) => {
      await settingsOpen(page);
      await bridge.call("sim.feed", { state: "stranger", kind: "deb" });
      await page.getByRole("button", { name: "Check for updates" }).click();
      await expect(page.locator("[data-update]")).toContainText("no valid signature");
    },
  },
  {
    name: "settings-update-retrying",
    hash: "#/settings",
    ready: async (page, bridge) => {
      await settingsOpen(page);
      await bridge.call("sim.feed", { state: "down", kind: "deb" });
      await page.getByRole("button", { name: "Check for updates" }).click();
      await expect(page.getByRole("button", { name: "Stop" })).toBeVisible();
    },
  },
  {
    name: "settings-key",
    hash: "#/settings",
    ready: async (page, bridge) => {
      await settingsOpen(page);
      await bridge.call("sim.pick_key", {});
      await page.getByRole("button", { name: "Add key…" }).click();
      await page.locator('[data-key="0807060504030201"]').getByRole("button", { name: "Details" }).click();
      await page.getByRole("button", { name: "Remove…" }).click();
      await page.locator('[data-panel="security"]').getByRole("switch").click();
      await page.getByRole("button", { name: "Make support bundle" }).click();
      await expect(page.locator("[data-bundle]")).toBeVisible();
    },
  },
  {
    name: "robot",
    hash: ROBOT,
    mockup: "robot.html",
    ready: async (page) => {
      await demoSettled(page);
      await expect(page.locator("[data-robot-device]")).toHaveCount(2);
    },
  },
  {
    name: "robot-quiet",
    hash: ROBOT,
    ready: async (page, bridge) => {
      await demoSettled(page);
      test.setTimeout(90_000);
      await bridge.call("sim.set_online", { id: DEMO.radio.id, online: false });
      // The robot drivers keep a connect's answer for twenty seconds.
      await expect(page.locator(`[data-robot-device="${DEMO.radio.id}"] .chip.off`)).toBeVisible({ timeout: 40_000 });
    },
  },
  {
    name: "systemcore",
    hash: SYSTEMCORE,
    mockup: "systemcore.html",
    ready: async (page) => {
      await demoSettled(page);
      await expect(page.locator('[data-panel="device"] dd').first()).toHaveText("SystemCore");
    },
  },
  {
    name: "devices",
    hash: "#/devices",
    mockup: "devices.html",
    ready: async (page) => {
      await demoSettled(page);
      await expect(page.locator(".tr.dev")).toHaveCount(7);
    },
  },
  {
    name: "devices-empty",
    hash: "#/devices",
    mockup: "devices-empty.html",
    scene: "empty",
    ready: async (page) => {
      await expect(page.locator(".empty-box")).toBeVisible();
    },
  },
  // The board page as the mockup draws it: a slot being written, a trial running, a fallback, a
  // pinned slot and a removal being confirmed in place, with the board's log beside them.
  {
    name: "board",
    hash: BOARD,
    mockup: "board.html",
    ready: async (page, bridge) => {
      await boardRead(page);
      for (const line of ["boot: slot-4 confirmed as default", "system: reboot requested from Atlas", "warn: camera: 2 frames dropped in the last 60 s", "boot: trial boot of slot-3, waiting for confirm", "update: slot-5: writing started"])
        await bridge.call("sim.node", { log: line });
      await page.locator('[data-slot="slot-3"]').getByRole("button", { name: "Trial" }).click();
      await expect(page.locator('[data-slot="slot-3"] .chip.warn')).toBeVisible();
      await bridge.call("sim.node", { hold_installs: true });
      await bridge.call("sim.pick_bundle", { slot: "slot-5", version: "2026.11.14" });
      await page.getByRole("button", { name: "Install…" }).click();
      await expect(page.locator("[data-install]")).toBeVisible();
      await page.locator('[data-slot="slot-1"]').getByRole("button", { name: "Remove" }).click();
      await expect(page.locator(".lines .ln").nth(4)).toBeVisible();
      await page.waitForTimeout(9000); // a minute of readings would be better; nine seconds draws a line
    },
  },
  {
    name: "board-idle",
    hash: BOARD,
    ready: async (page) => {
      await boardRead(page);
      // Locate is off with its reason (no board offers one); Reboot is on (the board's restart device).
      await expect(page.locator("header.top [data-not-offered]")).toHaveText("This board doesn't offer locate.");
      await expect(page.locator("header.top").getByRole("button", { name: "Reboot" })).toBeEnabled();
    },
  },
  {
    name: "board-hardware",
    hash: BOARD,
    ready: async (page) => {
      await boardRead(page);
      await page.locator('[data-panel="dock"] .tabbar').getByText("Hardware", { exact: true }).click();
      await expect(page.locator("[data-device]")).toHaveCount(7);
    },
  },
  // No board offers files yet: the tab says so in one sentence.
  {
    name: "board-files",
    hash: BOARD,
    ready: async (page) => {
      await boardRead(page);
      await page.locator('[data-panel="dock"] .tabbar').getByText("Files", { exact: true }).click();
      await expect(page.locator('[data-panel="dock"] .pane-files')).toHaveText("This board doesn't offer files yet.");
    },
  },
  // A board whose file has no restart device: Reboot is off beside Locate, with the reason in words.
  {
    name: "board-no-reboot",
    hash: BOARD,
    ready: async (page, bridge) => {
      await boardRead(page);
      await bridge.call("sim.node", { remove_device: "reboot" });
      await expect(page.locator("header.top").getByRole("button", { name: "Reboot" })).toBeDisabled();
      await expect(page.locator("header.top [data-not-offered]")).toHaveText("This board doesn't offer locate or reboot.");
    },
  },
  {
    name: "board-offline",
    hash: BOARD,
    ready: async (page, bridge) => {
      await boardRead(page);
      await bridge.call("sim.set_online", { id: DEMO.running.id, online: false });
      await expect(page.locator("header.top .chip.off")).toBeVisible();
    },
  },
  {
    name: "board-failed-install",
    hash: BOARD,
    ready: async (page, bridge) => {
      await boardRead(page);
      await bridge.call("sim.pick_bundle", { slot: "slot-9", version: "2026.12.1", board: "another-board" });
      await page.getByRole("button", { name: "Install…" }).click();
      await expect(page.locator('[data-install="failed"]')).toBeVisible();
    },
  },
  {
    name: "board-plain",
    hash: "#/devices",
    scene: "plain",
    ready: async (page) => {
      await page.locator("aside.side").getByText("board", { exact: true }).click();
      await expect(page.locator('[data-slots-state="none"]')).toBeVisible();
    },
  },
  // The flash flow. This build stops after the loader, so `flash` is drawn at that point; the
  // mockup shows a write in progress, which no library does yet.
  {
    name: "flash",
    hash: FLASH,
    mockup: "flash.html",
    ready: async (page, bridge) => {
      await flashReady(page, bridge);
      await page.getByRole("button", { name: "Send loader" }).click();
      await expect(page.locator('[data-flash-chip="loaded"]')).toBeVisible();
    },
  },
  {
    name: "flash-failed",
    hash: FLASH,
    mockup: "flash-failed.html",
    ready: async (page, bridge) => {
      await flashReady(page, bridge);
      await bridge.call("sim.rom", { silent: 400 });
      await page.getByRole("button", { name: "Send loader" }).click();
      await expect(page.locator('[data-panel="failure"] dl')).toContainText(/[6-9] so far/, { timeout: 25_000 });
      await page.waitForTimeout(1500); // part of the way through the wait, so the bar shows it
    },
  },
  {
    name: "flash-stopped",
    hash: FLASH,
    ready: async (page, bridge) => {
      await flashReady(page, bridge);
      await bridge.call("sim.rom", { silent: 400 });
      await page.getByRole("button", { name: "Send loader" }).click();
      await page.locator('[data-panel="failure"]').getByRole("button", { name: "Stop retrying" }).click();
      await expect(page.locator('[data-panel="failure"]')).toHaveAttribute("data-status", "stopped");
    },
  },
  {
    name: "flash-idle",
    hash: FLASH,
    ready: async (page) => {
      await demoSettled(page);
      await expect(page.locator('[data-panel="board"] .chip.acc')).toBeVisible();
    },
  },
  {
    name: "flash-ready",
    hash: FLASH,
    ready: flashReady,
  },
  {
    name: "flash-refused",
    hash: FLASH,
    ready: async (page, bridge) => {
      await demoSettled(page);
      await bridge.call("sim.pick_image", { broken: true, name: "notes.img" });
      await page.getByRole("button", { name: "Choose image…" }).click();
      await expect(page.locator('[data-panel="image"]')).toHaveAttribute("data-image", "refused");
    },
  },
  {
    name: "flash-none",
    hash: "#/flash",
    scene: "empty",
    ready: async (page) => {
      await expect(page.locator('[data-board="none"]')).toBeVisible();
    },
  },
  {
    name: "flash-disk",
    hash: DISK,
    mockup: "flash-disk.html",
    ready: async (page, bridge) => {
      await flashReady(page, bridge);
      await bridge.call("sim.disk", { stick: true });
      await expect(page.locator("[data-disk]")).toHaveCount(2);
      await page.locator("[data-foot]").getByRole("button", { name: "Erase and write…" }).click();
      await expect(page.locator("[data-foot] .q")).toBeVisible();
    },
  },
  {
    name: "flash-disk-idle",
    hash: DISK,
    ready: async (page) => {
      await demoSettled(page);
      await expect(page.locator("[data-disk]")).toHaveCount(1);
    },
  },
  {
    name: "flash-disk-writing",
    hash: DISK,
    ready: async (page, bridge) => {
      await diskReady(page, bridge);
      await bridge.call("sim.disk", { hold: true });
      await page.locator("[data-foot]").getByRole("button", { name: "Erase and write…" }).click();
      await page.locator("[data-foot]").getByRole("button", { name: "Erase and write", exact: true }).click();
      await expect(page.locator("[data-foot]")).toHaveAttribute("data-foot", "writing");
    },
  },
  {
    name: "flash-disk-done",
    hash: DISK,
    ready: async (page, bridge) => {
      await diskReady(page, bridge);
      await page.locator("[data-foot]").getByRole("button", { name: "Erase and write…" }).click();
      await page.locator("[data-foot]").getByRole("button", { name: "Erase and write", exact: true }).click();
      await expect(page.locator("[data-foot]")).toHaveAttribute("data-foot", "done");
    },
  },
  {
    name: "flash-disk-none",
    hash: "#/disk",
    scene: "empty",
    ready: async (page) => {
      await expect(page.locator('[data-panel="disks"] .empty-box')).toBeVisible();
    },
  },
];

test.describe("pictures", () => {
  test.skip(OUT === "", "set ATLAS_SCREENS_DIR to take pictures");
  for (const scheme of ["dark", "light"] as const) {
    for (const screen of SCREENS) {
      test.describe(() => {
        test.use({ scene: screen.scene ?? "demo" });
        test(`${screen.name} (${scheme})`, async ({ app, bridge }) => {
        mkdirSync(OUT, { recursive: true });
        await app.emulateMedia({ colorScheme: scheme });
        await app.evaluate((hash) => (location.hash = hash), screen.hash);
        await screen.ready(app, bridge);
        await app.evaluate(() => document.fonts.ready);
        await app.screenshot({ path: join(OUT, `${screen.name}-${scheme}.png`) });
        });
      });
      if (screen.mockup) {
        test(`${screen.name} mockup (${scheme})`, async ({ page }) => {
          mkdirSync(OUT, { recursive: true });
          await page.emulateMedia({ colorScheme: scheme });
          await page.goto(pathToFileURL(join(MOCKUPS, screen.mockup!)).href);
          await page.addStyleTag({ content: PLEX });
          await page.evaluate(() => document.fonts.ready);
          await page.screenshot({ path: join(OUT, `${screen.name}-${scheme}-mockup.png`) });
        });
      }
    }
  }
});
