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
    // A red test is explained from its own run: Playwright's page snapshot at the failure
    // (error-context.md), a screenshot, the bridge's request log and the app's log (see
    // bridge.ts); a test that passes keeps none. The full trace is off unless ATLAS_E2E_TRACE=1:
    // recorded for every test it cost 45% of the suite's CPU (2026-11-24, load ~85-165:
    // 196 CPU-s with it, 106 without, 83 tests at 4 workers).
    trace: process.env.ATLAS_E2E_TRACE === "1" ? "retain-on-failure" : "off",
    screenshot: "only-on-failure",
  },
});
