// Pictures of each screen and of its mockup, for comparing the two side by side. Not a test of
// anything: it runs only when ATLAS_SCREENS_DIR names where the pictures go
// (scripts/ui-gate.sh --screens DIR). The mockups are drawn with the app's own IBM Plex files,
// which is the font the design language names; without them a mockup falls back to the system
// font and the two pictures would differ in every glyph.
import { mkdirSync } from "node:fs";
import { join, resolve } from "node:path";
import { pathToFileURL } from "node:url";
import type { Page } from "@playwright/test";
import { demoSettled, expect, test, type Scene } from "./bridge";

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
export type Screen = { name: string; hash: string; ready: (page: Page) => Promise<void>; mockup?: string; scene?: Scene };

export const SCREENS: Screen[] = [
  {
    name: "devices",
    hash: "#/devices",
    mockup: "devices.html",
    ready: async (page) => {
      await demoSettled(page);
      await expect(page.locator(".tr.dev")).toHaveCount(6);
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
];

test.describe("pictures", () => {
  test.skip(OUT === "", "set ATLAS_SCREENS_DIR to take pictures");
  for (const scheme of ["dark", "light"] as const) {
    for (const screen of SCREENS) {
      test.describe(() => {
        test.use({ scene: screen.scene ?? "demo" });
        test(`${screen.name} (${scheme})`, async ({ app }) => {
        mkdirSync(OUT, { recursive: true });
        await app.emulateMedia({ colorScheme: scheme });
        await app.evaluate((hash) => (location.hash = hash), screen.hash);
        await screen.ready(app);
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
