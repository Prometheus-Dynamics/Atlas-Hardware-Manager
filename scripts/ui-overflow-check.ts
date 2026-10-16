// Checks the device page for text that's cut off and for an Overview that
// scrolls, against the browser mock (bun run dev in apps/desktop), at a
// large and a small laptop size, in ROBOT and PIT mode:
//
//   bun scripts/ui-overflow-check.ts [http://localhost:1423]
//
// A leaf element whose text is wider than its box fails unless it (or an
// ancestor) has a title, which says the rest on hover. The Overview's
// scroller must fit. Uses a headless Chrome over the DevTools protocol
// (CHROME, else Playwright's chrome-headless-shell in ~/.cache).
import { spawn } from "node:child_process";
import { mkdtempSync, readdirSync, readFileSync, existsSync, rmSync } from "node:fs";
import { homedir, tmpdir } from "node:os";
import { join } from "node:path";

const base = process.argv[2] ?? "http://localhost:1423";
const PAGE = "/device?key=raze%3A8f3a1c2d";
const SIZES = [
  [1440, 900],
  [1280, 800],
] as const;
const MODES = ["robot", "pit"] as const;

function chrome(): string {
  if (process.env.CHROME) return process.env.CHROME;
  const root = join(homedir(), ".cache/ms-playwright");
  const dirs = existsSync(root) ? readdirSync(root).filter((d) => d.startsWith("chromium_headless_shell-")).sort() : [];
  for (const dir of dirs.reverse()) {
    const bin = join(root, dir, "chrome-headless-shell-linux64/chrome-headless-shell");
    if (existsSync(bin)) return bin;
  }
  throw new Error("no headless Chrome: set CHROME");
}

const profile = mkdtempSync(join(tmpdir(), "atlas-overflow-"));
const proc = spawn(chrome(), ["--headless", "--remote-debugging-port=0", `--user-data-dir=${profile}`, "--no-first-run", "about:blank"], {
  stdio: "ignore",
});
const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

let port = "";
for (let i = 0; i < 100 && !port; i++) {
  await sleep(100);
  try {
    port = readFileSync(join(profile, "DevToolsActivePort"), "utf8").split("\n")[0];
  } catch {
    // Not up yet.
  }
}
if (!port) throw new Error("Chrome didn't start");
const pages = (await (await fetch(`http://127.0.0.1:${port}/json/list`)).json()) as { type: string; webSocketDebuggerUrl: string }[];
const ws = new WebSocket(pages.find((p) => p.type === "page")!.webSocketDebuggerUrl);
await new Promise((r) => (ws.onopen = r));
let next = 1;
const waiting = new Map<number, (v: unknown) => void>();
ws.onmessage = (e) => {
  const msg = JSON.parse(String(e.data));
  if (msg.id && waiting.has(msg.id)) {
    waiting.get(msg.id)!(msg.result ?? msg.error);
    waiting.delete(msg.id);
  }
};
const send = (method: string, params: Record<string, unknown> = {}) =>
  new Promise<any>((resolve) => {
    const id = next++;
    waiting.set(id, resolve);
    ws.send(JSON.stringify({ id, method, params }));
  });
const evaluate = async (expression: string) =>
  (await send("Runtime.evaluate", { expression, awaitPromise: true, returnByValue: true })).result?.value;

const CHECK = `(() => {
  const clipped = [];
  for (const el of document.querySelectorAll('[role=tabpanel] *, header.head *')) {
    if (el.children.length || !el.textContent.trim()) continue;
    const cs = getComputedStyle(el);
    if (cs.display === 'none' || cs.visibility === 'hidden') continue;
    if (el.scrollWidth > el.clientWidth + 1 && !el.closest('[title]')) clipped.push(el.textContent.trim().slice(0, 60));
  }
  const scrollers = [...document.querySelectorAll('.scroller')].filter((s) => s.scrollHeight > s.clientHeight + 1).map((s) => s.scrollHeight - s.clientHeight);
  return { clipped, overflowPx: scrollers[0] ?? 0 };
})()`;

let failed = 0;
try {
  await send("Runtime.enable");
  for (const [width, height] of SIZES) {
    await send("Emulation.setDeviceMetricsOverride", { width, height, deviceScaleFactor: 1, mobile: false });
    for (const mode of MODES) {
      await send("Page.navigate", { url: `${base}/` });
      await sleep(800);
      await evaluate(`localStorage.setItem('atlas.mode', '${mode}')`);
      await send("Page.navigate", { url: `${base}${PAGE}` });
      await sleep(4000);
      const r = await evaluate(CHECK);
      const ok = r && r.clipped.length === 0 && r.overflowPx === 0;
      if (!ok) failed += 1;
      console.log(`${ok ? "ok  " : "FAIL"} ${width}x${height} ${mode}: ${r ? `${r.overflowPx}px past the window; cut off without a title: ${JSON.stringify(r.clipped)}` : "no result"}`);
    }
  }
} finally {
  ws.close();
  proc.kill();
  rmSync(profile, { recursive: true, force: true });
}
process.exit(failed ? 1 : 0);
