// The UI gate (e2e/ui-gate.spec.ts): the built UI, served by `vite preview`,
// against the browser mock, in Chromium (the only engine with the Layout
// Instability API). Screenshots are compared with baselines made in the
// Playwright image, so run it through scripts/ui-gate.sh, which uses that
// image here and in CI alike.
import { defineConfig } from "@playwright/test";

const port = 4173;

export default defineConfig({
  testDir: ".",
  outputDir: "../../../target/ui-gate",
  snapshotPathTemplate: "{testDir}/screens/{arg}{ext}",
  fullyParallel: true,
  workers: process.env.CI ? 4 : 6,
  retries: 0,
  reporter: [["list"], ["html", { open: "never", outputFolder: "../../../target/ui-gate-report" }]],
  timeout: 90_000,
  expect: { toHaveScreenshot: { maxDiffPixelRatio: 0.002, animations: "disabled", caret: "hide" } },
  use: {
    baseURL: process.env.ATLAS_UI_URL ?? `http://127.0.0.1:${port}`,
    browserName: "chromium",
    colorScheme: "dark",
    deviceScaleFactor: 1,
    launchOptions: process.env.CHROME ? { executablePath: process.env.CHROME } : {},
  },
  webServer: process.env.ATLAS_UI_URL
    ? undefined
    : { command: `node node_modules/vite/bin/vite.js preview --host 127.0.0.1 --port ${port} --strictPort`, port, reuseExistingServer: false, cwd: ".." },
});
