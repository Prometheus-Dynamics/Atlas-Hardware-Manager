// The click tests: the built UI, served by the UI bridge (the real core with the simulated
// devices), in headless Chromium. Run through scripts/ui-gate.sh, which builds both first.
import { defineConfig } from "@playwright/test";

export default defineConfig({
  testDir: ".",
  outputDir: process.env.ATLAS_E2E_OUT ?? "../test-results",
  fullyParallel: true,
  workers: 4,
  retries: 0,
  reporter: [["list"]],
  timeout: 30_000,
  expect: { timeout: 8_000 },
  use: {
    browserName: "chromium",
    colorScheme: "dark",
    deviceScaleFactor: 1,
    viewport: { width: 1320, height: 860 },
    // A red test is explained from its own run: its trace is kept (beside the bridge's request
    // log and the app's log, see bridge.ts), and a test that passes keeps none.
    trace: "retain-on-failure",
  },
});
