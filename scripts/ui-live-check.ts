// The UI gate's live half: the real app against real devices. Clicks
// through every page in the sidebar, every device page and each of its tabs
// in a running Atlas, and fails on a page that shows the error page or a
// card that failed, and on UI-ERROR lines that land in atlas.log meanwhile.
//
//   WEBKIT_INSPECTOR_HTTP_SERVER=127.0.0.1:9333 atlas-app     (Linux)
//   bun scripts/ui-live-check.ts [127.0.0.1:9333] [path/to/atlas.log]
//
// It drives the app over WebKitGTK's remote inspector, the way a user's
// clicks would (element.click()), so it needs no test build. For a test
// instance beside your own Atlas, give it its own data dir and session bus:
// XDG_DATA_HOME=<dir> dbus-run-session -- atlas-app.
import { existsSync, readFileSync, statSync } from "node:fs";
import { homedir } from "node:os";
import { join } from "node:path";

const server = process.argv[2] ?? "127.0.0.1:9333";
const logPath = process.argv[3] ?? join(process.env.XDG_DATA_HOME ?? join(homedir(), ".local/share"), "atlas/atlas.log");
const SETTLE_MS = 3000;

const ws = new WebSocket(`ws://${server}/socket/1/1/WebPage`);
let target = "";
let next = 1;
const waiting = new Map<number, (msg: { result?: { result?: { value?: unknown; objectId?: string; className?: string }; wasThrown?: boolean } }) => void>();

function send(method: string, params: object): Promise<{ result?: { result?: { value?: unknown; objectId?: string; className?: string }; wasThrown?: boolean } }> {
  const id = next++;
  ws.send(JSON.stringify({ id: next++, method: "Target.sendMessageToTarget", params: { targetId: target, message: JSON.stringify({ id, method, params }) } }));
  return new Promise((resolve) => waiting.set(id, resolve));
}

/** Evaluates an expression in the app, awaiting it if it's a promise. */
async function run<T>(expression: string): Promise<T> {
  const first = await send("Runtime.evaluate", { expression });
  const remote = first.result?.result;
  if (first.result?.wasThrown) throw new Error(`threw: ${JSON.stringify(remote)}`);
  if (!remote?.objectId) return remote?.value as T;
  const value =
    remote.className === "Promise"
      ? await send("Runtime.awaitPromise", { promiseObjectId: remote.objectId, returnByValue: true })
      : await send("Runtime.callFunctionOn", { objectId: remote.objectId, functionDeclaration: "function(){return this}", returnByValue: true });
  return value.result?.result?.value as T;
}

await new Promise<void>((resolve, reject) => {
  ws.onerror = () => reject(new Error(`no Atlas inspector at ${server}: start atlas-app with WEBKIT_INSPECTOR_HTTP_SERVER=${server}`));
  ws.onmessage = (e) => {
    const msg = JSON.parse(String(e.data));
    if (msg.method === "Target.targetCreated" && !target) {
      target = msg.params.targetInfo.targetId;
      resolve();
    } else if (msg.method === "Target.dispatchMessageFromTarget") {
      const inner = JSON.parse(msg.params.message);
      waiting.get(inner.id)?.(inner);
      waiting.delete(inner.id);
    }
  };
});

const logStart = existsSync(logPath) ? statSync(logPath).size : 0;
const problems: string[] = [];
const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));
// What a page shows when it, or one of its cards, failed (+error.svelte, Guard.svelte).
const check = (where: string) =>
  run<string | null>(`(document.querySelector("main")?.innerText.match(/[^\\n]*(hit a bug|Internal Error)[^\\n]*/) ?? [null])[0]`).then((hit) => {
    console.log(`${hit ? "FAIL" : "ok  "} ${where}${hit ? `: ${hit}` : ""}`);
    if (hit) problems.push(`${where}: ${hit}`);
  });
const click = (selector: string) => run<boolean>(`(() => { const a = document.querySelector(${JSON.stringify(selector)}); a?.click(); return !!a; })()`);

const pages = await run<string[]>(`[...new Set([...document.querySelectorAll("nav a[href^='/']")].map((a) => a.getAttribute("href")).filter((h) => !h.startsWith("/device?")))]`);
for (const href of pages) {
  await click(`nav a[href="${href}"]`);
  await sleep(SETTLE_MS);
  await check(href);
}

await click(`nav a[href="/devices"]`);
await sleep(SETTLE_MS);
const devices = await run<string[]>(`[...new Set([...document.querySelectorAll("a[href^='/device?key=']")].map((a) => a.getAttribute("href")))]`);
for (const href of devices) {
  await click(`a[href="${href}"]`);
  await sleep(SETTLE_MS);
  await check(href);
  const tabs = await run<string[]>(`[...document.querySelectorAll("main [role=radiogroup][aria-label='Device sections'] [role=radio]")].map((b) => b.innerText.trim()).filter(Boolean)`);
  for (const tab of tabs) {
    await run(`[...document.querySelectorAll("main [role=radiogroup][aria-label='Device sections'] [role=radio]")].find((b) => b.innerText.trim() === ${JSON.stringify(tab)})?.click()`);
    await sleep(SETTLE_MS);
    await check(`${href} · ${tab}`);
  }
  await click(`nav a[href="/devices"]`);
  await sleep(1000);
}

const logged = existsSync(logPath)
  ? readFileSync(logPath, "utf8")
      .slice(logStart)
      .split("\n")
      .filter((line) => line.includes("UI-ERROR"))
  : [];
for (const line of logged) problems.push(`atlas.log: ${line.slice(0, 400)}`);
console.log(`\n${pages.length} pages, ${devices.length} devices: ${problems.length ? `${problems.length} problems` : "clean"}`);
for (const p of problems) console.log(`  ${p}`);
ws.close();
process.exit(problems.length ? 1 : 0);
