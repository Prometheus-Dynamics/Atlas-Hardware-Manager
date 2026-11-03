// The UI gate's overflow half: every page, at a desktop and the smallest
// window, fails on content that doesn't fit its box. That's text cut off
// with nothing that says the rest (a title on it or around it), a page
// or card that scrolls sideways, and a child that spills out of its card.
// The device page's Overview must also fit the window without scrolling,
// in ROBOT and in PIT mode, at the two laptop sizes. Atlas is dark-only.
import { expect, test, type Page } from "@playwright/test";

const ROUTES = [
  ["overview", "/"],
  ["devices", "/devices"],
  ["device", "/device?key=raze%3A8f3a1c2d"],
  ["monitor", "/monitor"],
  ["networktables", "/networktables"],
  ["jobs", "/jobs"],
  ["releases", "/releases"],
  ["robots", "/robots"],
  ["settings", "/settings"],
] as const;
const SIZES = [
  ["desktop", 1440, 900],
  ["narrow", 1024, 680],
] as const;

async function open(page: Page, path: string) {
  await page.goto(path, { waitUntil: "domcontentloaded" });
  await page.locator("main").first().waitFor();
  await page.waitForLoadState("networkidle");
  // The sim keeps finding devices for a few seconds.
  await page.waitForTimeout(2500);
}

/** What doesn't fit, as one line each. */
const misfits = (page: Page) =>
  page.evaluate(() => {
    const out: string[] = [];
    const name = (el: Element) => {
      const cls = typeof el.className === "string" ? el.className.split(/\s+/).filter(Boolean).slice(0, 2).join(".") : "";
      return `${el.tagName.toLowerCase()}${cls ? "." + cls : ""} "${(el.textContent ?? "").trim().replace(/\s+/g, " ").slice(0, 40)}"`;
    };
    const shown = (el: Element) => {
      const cs = getComputedStyle(el);
      const box = el.getBoundingClientRect();
      return cs.display !== "none" && cs.visibility !== "hidden" && box.width > 0 && box.height > 0;
    };
    const root = document.scrollingElement ?? document.documentElement;
    if (root.scrollWidth > innerWidth + 1) out.push(`the page scrolls sideways by ${root.scrollWidth - innerWidth} px`);
    for (const el of document.querySelectorAll<HTMLElement>("body *")) {
      if (!shown(el) || el.closest("[data-live], svg, canvas, video, [role=dialog]")) continue;
      const cs = getComputedStyle(el);
      // Sideways scrolling inside the page (a table wider than its card).
      if (/(auto|scroll)/.test(cs.overflowX) && el.scrollWidth > el.clientWidth + 1) {
        out.push(`${name(el)} scrolls sideways by ${el.scrollWidth - el.clientWidth} px`);
      }
      // Text wider than its box, with no title to say the rest.
      const leaf = el.children.length === 0 && (el.textContent ?? "").trim() !== "";
      if (leaf && el.scrollWidth > el.clientWidth + 1 && !el.closest("[title]") && cs.display !== "inline") {
        out.push(`${name(el)} is cut off by ${el.scrollWidth - el.clientWidth} px`);
      }
    }
    // Children outside their card, sideways (floating popovers excepted).
    for (const card of document.querySelectorAll<HTMLElement>(".panel, section.glass, .glass-layer")) {
      if (!shown(card)) continue;
      const edge = card.getBoundingClientRect();
      for (const el of card.querySelectorAll<HTMLElement>("*")) {
        if (!shown(el) || el.closest("[data-live], [role=dialog], [role=tooltip], [role=menu]")) continue;
        let floating = false;
        for (let up: HTMLElement | null = el; up && up !== card; up = up.parentElement) {
          if (/(absolute|fixed)/.test(getComputedStyle(up).position)) floating = true;
        }
        if (floating) continue;
        const box = el.getBoundingClientRect();
        const over = Math.max(box.right - edge.right, edge.left - box.left);
        if (over > 1) out.push(`${name(el)} sticks out of ${name(card)} by ${Math.round(over)} px`);
      }
    }
    return [...new Set(out)];
  });

for (const [name, path] of ROUTES) {
  for (const [size, width, height] of SIZES) {
    test(`${name} @ ${size}: everything fits its box`, async ({ page }) => {
      await page.setViewportSize({ width, height });
      await open(page, path);
      expect(await misfits(page)).toEqual([]);
    });
  }
}

for (const [width, height] of [
  [1440, 900],
  [1280, 800],
] as const) {
  for (const mode of ["robot", "pit"] as const) {
    test(`device Overview @ ${width}x${height} ${mode}: fits without scrolling`, async ({ page }) => {
      await page.setViewportSize({ width, height });
      await page.addInitScript((m) => localStorage.setItem("atlas.mode", m), mode);
      await open(page, "/device?key=raze%3A8f3a1c2d");
      const over = await page.evaluate(() =>
        [...document.querySelectorAll(".scroller")].map((s) => s.scrollHeight - s.clientHeight).find((d) => d > 1) ?? 0,
      );
      expect(over, "px of the Overview below the window").toBe(0);
    });
  }
}
