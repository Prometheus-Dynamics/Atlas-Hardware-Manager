import js from "@eslint/js";
import pluginSvelte from "eslint-plugin-svelte";
import globals from "globals";
import tseslint from "typescript-eslint";

const ignores = [
  ".svelte-kit/**",
  "build/**",
  "dist/**",
  "src/lib/ts-bindings/**",
  "src-tauri/target/**",
];

const svelteRecommended = pluginSvelte.configs["flat/recommended"];

export default [
  {
    ignores,
  },
  js.configs.recommended,
  ...tseslint.configs.recommended,
  ...(Array.isArray(svelteRecommended) ? svelteRecommended : [svelteRecommended]),
  {
    files: ["**/*.svelte"],
    languageOptions: {
      parserOptions: {
        parser: tseslint.parser,
        extraFileExtensions: [".svelte"],
      },
    },
  },
  {
    files: ["**/*.{js,ts,svelte}"] ,
    languageOptions: {
      globals: {
        ...globals.browser,
        ...globals.node,
      },
    },
  },
];
