// The UI gate: every page, at a desktop and the smallest window, against the
// browser mock. A page fails on a console error, an uncaught exception, an
// HTTP status of 500 or more, or the error page; on a screenshot that differs
// from its baseline; on device lists that reorder between polls; and, for
// every button, tab and expander on it, on a click that moves anything other
// than what was clicked or what sits below it (an accordion may only push
// down), measured with the Layout Instability API.
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
// A fixed start, so times and the mock's data render the same every run.
const T0 = new Date("2026-06-01T12:00:00Z").getTime();
// Controls the sweep leaves alone: they leave the page or end the session.
// The UI scale buttons move everything, as they should.
const SKIP = /quit|sign out|restart atlas|install update|open (the )?(log|data) folder|report a bug|^back\b|^\d+ ?%$/i;
// The mock refreshes devices every 5 s at most (live.svelte.ts STREAM_POLL_MS).
const POLL_MS = 5000;

type Shift = { value: number; at: number; sources: { node: string | null; inside: boolean; below: boolean; dx: number; dy: number }[] };
declare global {
  interface Window {
    __shifts: Shift[];
    __clicked: Element | null;
    __box: DOMRect | null;
    __column: DOMRect | null;
  }
}

/** Seeded Math.random, and a layout-shift recorder that judges each moved
 *  node against the element clicked last. */
function instrument(page: Page) {
  return page.addInitScript(() => {
    let seed = 0x5eed;
    Math.random = () => {
      seed = (seed + 0x6d2b79f5) | 0;
      let t = Math.imul(seed ^ (seed >>> 15), 1 | seed);
      t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
      return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
    };
    window.__shifts = [];
    window.__clicked = null;
    window.__box = null;
    window.__column = null;
    const describe = (n: Node | null): string | null => {
      if (n && !(n instanceof Element)) return n.parentElement ? `text in ${describe(n.parentElement)}` : n.nodeName;
      if (!(n instanceof Element)) return null;
      const cls = typeof n.className === "string" ? n.className.split(/\s+/).slice(0, 3).join(".") : "";
      return `${n.tagName.toLowerCase()}${cls ? "." + cls : ""} "${(n.textContent ?? "").trim().slice(0, 40)}"`;
    };
    new PerformanceObserver((list) => {
      for (const entry of list.getEntries() as unknown as {
        value: number;
        startTime: number;
        sources: { node: Node | null; previousRect: DOMRectReadOnly; currentRect: DOMRectReadOnly }[];
      }[]) {
        // Where the control was when it was clicked: it may be gone now.
        const clicked = window.__clicked;
        const box = window.__box;
        const column = window.__column;
        window.__shifts.push({
          value: entry.value,
          at: entry.startTime,
          // Text that moves inside its own box (a right-aligned number gaining
          // a digit) isn't layout moving, and [data-live] marks a chart or
          // bar whose parts move with the data on purpose.
          sources: entry.sources
            .filter((s) => s.node instanceof Element && !s.node.closest("[data-live]"))
            .map((s) => ({
            node: describe(s.node),
            // The sidebar answers what a click started (a job, an update
            // found): its own stillness is the idle check's to judge.
            inside: !!(clicked && s.node && (clicked.contains(s.node) || s.node.contains(clicked) || (s.node as Element).closest("nav"))),
            // Scrolled into view from under a collapse: an empty box before.
            below:
              (s.previousRect.width === 0 && s.previousRect.height === 0) ||
              (!!box &&
              !!column &&
              s.previousRect.top >= box.bottom - 1 &&
              s.previousRect.left < column.right &&
              s.previousRect.right > column.left),
            // Pushed out of its scroller's view, or into it, a node reports an
            // empty box on that side.
            dx:
              (s.currentRect.width === 0 && s.currentRect.height === 0) || (s.previousRect.width === 0 && s.previousRect.height === 0)
                ? 0
                : Math.round(s.currentRect.left - s.previousRect.left),
            dy: Math.round(s.currentRect.top - s.previousRect.top),
          })),
        });
      }
    }).observe({ type: "layout-shift", buffered: false });
  });
}

/** Fails the test on console errors, uncaught exceptions and 5xx responses. */
function guard(page: Page): string[] {
  const problems: string[] = [];
  // A network change on the machine running the gate isn't the page's.
  page.on("console", (m) => m.type() === "error" && !m.text().includes("ERR_NETWORK_CHANGED") && problems.push(`console: ${m.text()}`));
  page.on("pageerror", (e) => problems.push(`exception: ${e.message}`));
  page.on("response", (r) => r.status() >= 500 && problems.push(`HTTP ${r.status()}: ${r.url()}`));
  return problems;
}

/** Opens a page. With the clock paused, the page's timers run in fake time,
 *  so it renders the same every run. */
async function open(page: Page, path: string, paused = false) {
  await page.goto(path, { waitUntil: "domcontentloaded" });
  if (paused) {
    // 6 s of the page's time in quarter-second steps, each a moment of real
    // time for its promises to settle.
    for (let i = 0; i < 24; i++) {
      await page.clock.fastForward(250);
      await page.waitForTimeout(25);
    }
  } else {
    await page.locator("main").first().waitFor();
    await page.waitForLoadState("networkidle");
  }
  await expect(page.getByText("This page hit a bug")).toHaveCount(0);
}

/** What moves while nobody touches the page: live numbers that change width
 *  and push their neighbours, a list that reflows on a poll. */
async function idleShifts(page: Page, ms: number): Promise<string[]> {
  await page.evaluate(() => {
    window.__shifts = [];
    window.__clicked = null;
  });
  await page.waitForTimeout(ms);
  return page.evaluate(() => [...new Set(window.__shifts.flatMap((s) => s.sources.map((src) => src.node ?? "?")))]);
}

/** Waits for the sim's discovery to finish: no device appears for 3 s. */
async function settle(page: Page) {
  let last = -1;
  for (let still = 0, i = 0; still < 12 && i < 120; i++) {
    const devices = await page.locator('a[href^="/device?key="], [aria-label^="Open "]').count();
    still = devices === last ? still + 1 : 0;
    last = devices;
    await page.waitForTimeout(250);
  }
}

/** Marks the controls to click with data-gate="<n>", in order: expanders and
 *  tabs first, then buttons; two of each kind of row control (the rest would
 *  say the same); links left out (they leave the page). Returns how many. */
const pickControls = (page: Page) =>
  page.evaluate(
    ({ skip, cap }) => {
      const skipRe = new RegExp(skip, "i");
      document.querySelectorAll("[data-gate]").forEach((el) => el.removeAttribute("data-gate"));
      const all = [...document.querySelectorAll<HTMLElement>('main [aria-expanded], main summary, main [role="tab"], main [role="radio"], main button:not([disabled])')];
      const rank = (el: Element) => (el.matches("[aria-expanded], summary") ? 0 : el.matches('[role="tab"], [role="radio"]') ? 1 : 2);
      const seen = new Map<string, number>();
      const picked: HTMLElement[] = [];
      for (const el of [...new Set(all)].sort((a, b) => rank(a) - rank(b))) {
        const box = el.getBoundingClientRect();
        if (box.width === 0 || box.height === 0) continue;
        const label = (el.getAttribute("aria-label") ?? el.innerText ?? "").trim().replace(/\s+/g, " ").slice(0, 60);
        if (skipRe.test(label)) continue;
        const kind = el.closest("li, tr, [role=row]") ? `row:${label.split(" ")[0]}` : `one:${label}`;
        const n = seen.get(kind) ?? 0;
        if (n >= (kind.startsWith("row:") ? 2 : 1)) continue;
        seen.set(kind, n + 1);
        el.setAttribute("data-gate-label", label);
        picked.push(el);
        if (picked.length >= cap) break;
      }
      picked.forEach((el, i) => el.setAttribute("data-gate", String(i)));
      return picked.length;
    },
    { skip: SKIP.source, cap: 40 },
  );

/** Device rows in document order: what a poll must not shuffle. */
const deviceOrder = (page: Page) =>
  page.evaluate(() =>
    [...document.querySelectorAll('[aria-label^="Open "], a[href^="/device?key="]')].map(
      (e) => e.getAttribute("aria-label") ?? e.getAttribute("href"),
    ),
  );

for (const [name, path] of ROUTES) {
  for (const [size, width, height] of SIZES) {
    test(`${name} @ ${size}: loads clean, matches its screenshot, keeps its order`, async ({ page }) => {
      const problems = guard(page);
      await page.setViewportSize({ width, height });
      // All of it in the page's own time, stepped from a fixed start, so it
      // shows the same time and data every run.
      await page.clock.install({ time: T0 });
      await page.clock.pauseAt(T0 + 100);
      await instrument(page);
      await open(page, path, true);
      // The sim keeps finding devices for a while.
      for (let i = 0, last = ""; i < 12; i++) {
        await page.clock.fastForward(5000);
        await page.waitForTimeout(150);
        const now = (await deviceOrder(page)).join();
        if (now === last) break;
        last = now;
      }
      // Chart lines ([data-live]) are masked: their samples land a little
      // differently run to run. Their frames, axes and legends still count.
      await expect(page).toHaveScreenshot(`${name}-${size}.png`, { fullPage: false, mask: [page.locator("[data-live]")] });

      const first = await deviceOrder(page);
      for (let poll = 0; poll < 3; poll++) {
        await page.clock.fastForward(POLL_MS);
        await page.waitForTimeout(100);
        expect(await deviceOrder(page), `device order after poll ${poll + 1}`).toEqual(first);
      }
      expect(problems).toEqual([]);
    });
  }

  test(`${name}: every control clicks clean and moves only what it should`, async ({ page }) => {
    const problems = guard(page);
    test.setTimeout(150_000);
    await page.setViewportSize({ width: 1440, height: 900 });
    await instrument(page);
    await open(page, path);
    await settle(page);
    const jitter = await idleShifts(page, 3000);
    // The sweep runs on a paused clock, so live data holds still while it
    // judges what each click moved; a click's own work runs in fake time.
    await page.clock.install();
    await page.clock.pauseAt(Date.now() + 1000);
    const total = await pickControls(page);
    const failures: string[] = [];
    for (let i = 0; i < total; i++) {
      const control = page.locator(`[data-gate="${i}"]`);
      if (!(await control.isVisible().catch(() => false))) continue;
      const label = (await control.getAttribute("data-gate-label")) ?? "";
      // Tabs, radios and pressed filters switch what's shown: what's below
      // them may change entirely. Everything else may only push down, in
      // its own column (the card it's in; a toolbar's is the page).
      // Into view first: a click's own scrolling isn't the click's layout.
      await control.evaluate((el) => el.scrollIntoView({ block: "center", inline: "center" })).catch(() => undefined);
      await page.waitForTimeout(60);
      const switches = await control.evaluate((el) => {
        window.__shifts = [];
        window.__clicked = el;
        window.__box = el.getBoundingClientRect();
        window.__column = (el.closest("section, article, .panel, .glass") ?? el.closest("main") ?? el).getBoundingClientRect();
        return el.matches('[role="tab"], [role="radio"], [aria-pressed]');
      });
      const before = page.url();
      await control.click({ timeout: 1000 }).catch(() => undefined);
      await page.clock.fastForward(400);
      // Long enough for the open and close animations (≤ 300 ms in app.css).
      await page.waitForTimeout(320);
      await expect(page.getByText("This page hit a bug"), `after clicking "${label}"`).toHaveCount(0);
      if (page.url() === before) {
        const moved = await page.evaluate(() =>
          window.__shifts.flatMap((s) =>
            s.sources
              .filter((src) => !src.inside && !(src.below && src.dx === 0))
              .map((src) => `${src.node} (${src.below ? "below" : "beside or above"}, ${src.dx},${src.dy} px)`),
          ),
        );
        if (moved.length && !switches) failures.push(`"${label}" moved ${[...new Set(moved)].join(", ")}`);
      }
      await page.keyboard.press("Escape");
      if (page.url() !== before) {
        await open(page, path, true);
        await pickControls(page);
      } else {
        await page.clock.fastForward(300);
        await page.waitForTimeout(250);
      }
    }
    expect(jitter, "content that moved on its own, with nobody clicking").toEqual([]);
    expect(failures, "clicks that moved content outside the clicked control or above it").toEqual([]);
    expect(problems).toEqual([]);
  });
}
