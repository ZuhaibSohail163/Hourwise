import { defineConfig, devices } from "@playwright/test";

/**
 * End-to-end tests. Run them with `npm run test:e2e` from the repo root, which starts an
 * isolated Docker stack (see docker-compose.e2e.yml) and points the tests at it.
 */
export default defineConfig({
  testDir: "./e2e",
  fullyParallel: false,
  workers: 1,
  retries: process.env.CI ? 1 : 0,
  reporter: [["list"]],
  timeout: 60_000,
  use: {
    baseURL: process.env.E2E_BASE_URL ?? "http://localhost:3100",
    trace: "retain-on-failure",
    screenshot: "only-on-failure",
  },
  projects: [{ name: "chromium", use: { ...devices["Desktop Chrome"] } }],
});
