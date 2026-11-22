// Where the person is. The route lives in the URL's hash, so Back works and a page can be
// opened directly. A route is a page name and the rest of the path (a device id, a robot name).

import { breadcrumb } from "./app.svelte";

export type PageName = "devices" | "board" | "flash" | "disk" | "robot" | "systemcore" | "settings";
export type Route = { page: PageName; arg: string };

const PAGES: PageName[] = ["devices", "board", "flash", "disk", "robot", "systemcore", "settings"];

function parse(hash: string): Route {
  const [, page = "", ...rest] = hash.replace(/^#/, "").split("/");
  const arg = decodeURIComponent(rest.join("/"));
  return PAGES.includes(page as PageName) ? { page: page as PageName, arg } : { page: "devices", arg: "" };
}

export const route = $state<Route>(parse(location.hash));

window.addEventListener("hashchange", () => {
  const next = parse(location.hash);
  route.page = next.page;
  route.arg = next.arg;
  breadcrumb(`opened ${next.page}${next.arg ? ` ${next.arg}` : ""}`);
});

export function href(page: PageName, arg = ""): string {
  return arg ? `#/${page}/${encodeURIComponent(arg)}` : `#/${page}`;
}

export function go(page: PageName, arg = "") {
  location.hash = href(page, arg).slice(1);
}
