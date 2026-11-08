import adapter from "@sveltejs/adapter-static";
import { sveltekit } from "@sveltejs/kit/vite";
import { vitePreprocess } from "@sveltejs/vite-plugin-svelte";
import tailwindcss from "@tailwindcss/vite";
import { createHash } from "node:crypto";
import { readFileSync } from "node:fs";
import { defineConfig, type Plugin } from "vite";

const host = process.env.TAURI_DEV_HOST;

/**
 * `vite preview` (the UI gate's server) sends the app's own CSP, from
 * tauri.conf.json, the way Tauri does: with a hash for each inline script.
 * A source the real app would refuse (a camera frame's blob: URL, once) then
 * fails the gate as a console error instead of shipping.
 */
function realCsp(): Plugin {
  const conf = JSON.parse(readFileSync(new URL("./src-tauri/tauri.conf.json", import.meta.url), "utf8"));
  const csp: string = conf.app.security.csp;
  const withScripts = (html: string) => {
    const hashes = [...html.matchAll(/<script(?![^>]*\bsrc=)[^>]*>([\s\S]*?)<\/script>/g)].map(
      ([, body]) => `'sha256-${createHash("sha256").update(body).digest("base64")}'`,
    );
    return `${csp}; script-src 'self' ${hashes.join(" ")}`;
  };
  return {
    name: "atlas-real-csp",
    configurePreviewServer(server) {
      server.middlewares.use((req, res, next) => {
        // Only pages; scripts, styles and images pass untouched.
        if (!req.headers.accept?.includes("text/html")) return next();
        // Uncompressed, so the hashes are of the script text the browser runs.
        delete req.headers["accept-encoding"];
        // Hold the page back until it is whole, then hash what it inlines.
        const end = res.end.bind(res);
        const writeHead = res.writeHead.bind(res);
        const held: Buffer[] = [];
        let head: Parameters<typeof res.writeHead> | undefined;
        res.writeHead = ((...args: Parameters<typeof res.writeHead>) => {
          head = args;
          return res;
        }) as typeof res.writeHead;
        res.write = ((chunk: unknown) => {
          held.push(Buffer.from(chunk as Buffer));
          return true;
        }) as typeof res.write;
        res.end = ((chunk?: unknown) => {
          if (chunk && typeof chunk !== "function") held.push(Buffer.from(chunk as Buffer));
          const body = Buffer.concat(held);
          const headers = (head?.find((h) => h && typeof h === "object") ?? {}) as Record<string, unknown>;
          const type = String(headers["Content-Type"] ?? res.getHeader("Content-Type") ?? "");
          if (type.includes("text/html")) {
            res.setHeader("Content-Security-Policy", withScripts(body.toString("utf8")));
          }
          if (head) writeHead(...head);
          return end(body);
        }) as typeof res.end;
        next();
      });
    },
  };
}

export default defineConfig({
  plugins: [
    realCsp(),
    tailwindcss(),
    sveltekit({
      preprocess: vitePreprocess(),
      // A single-page app: Tauri serves index.html for every route.
      adapter: adapter({ fallback: "index.html" }),
    }),
  ],
  clearScreen: false,
  server: {
    port: 1422,
    strictPort: true,
    host: host || false,
    hmr: host ? { protocol: "ws", host, port: 1423 } : undefined,
    watch: { ignored: ["**/src-tauri/**"] },
    // Package data the app reads at build time (boardCase.ts).
    fs: { allow: ["../../devices"] },
  },
});
