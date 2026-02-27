import fs from "node:fs";
import path from "node:path";
import type { Config } from "tailwindcss";
import { skeleton } from "@skeletonlabs/tw-plugin";

const heliosTheme = (() => {
  const themePath = path.resolve("src/themes/helios.css");
  const css = fs.readFileSync(themePath, "utf8");
  const blockMatch = css.match(/\[data-theme=['"]helios['"]\]\s*\{([\s\S]*?)\n\}/);

  if (!blockMatch) {
    throw new Error("Unable to locate helios theme in src/themes/helios.css");
  }

  const declarations = blockMatch[1];
  const entries = Array.from(
    declarations.matchAll(/--([a-z0-9-]+):\s*([^;]+);/gi),
  ).map(([, name, value]) => [`--${name}`, value.trim()]);

  return {
    name: "helios",
    properties: Object.fromEntries(entries),
  };
})();

const config = {
  darkMode: "class",
  content: ["./src/**/*.{html,js,svelte,ts}"],
  theme: {
    extend: {
      fontFamily: {
        geologica: ["Geologica", "sans-serif"],
      },
    },
  },
  plugins: [
    skeleton({
      themes: {
        preset: ["skeleton"],
        custom: [heliosTheme],
      },
    }),
  ],
} satisfies Config;

export default config;
