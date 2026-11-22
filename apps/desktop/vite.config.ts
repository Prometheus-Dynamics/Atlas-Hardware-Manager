import { defineConfig } from "vite";
import { svelte } from "@sveltejs/vite-plugin-svelte";

// The UI is a plain single page: no server rendering, no router library. `dist/` is what Tauri
// embeds and what the UI bridge serves.
export default defineConfig({
  plugins: [svelte()],
  clearScreen: false,
  build: { outDir: "dist", emptyOutDir: true, target: "es2022", sourcemap: false },
  server: { port: 1422, strictPort: true },
});
