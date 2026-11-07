import { defineConfig, devices } from "@playwright/test";

const vitePort = process.env.PLAYWRIGHT_VITE_PORT ?? "1422";

export default defineConfig({
  testDir: "./tests",
  fullyParallel: true,
  forbidOnly: !!process.env.CI,
  retries: process.env.CI ? 2 : 0,
  // The CI runner has 4 vCPUs; the suite is fullyParallel and one Vite dev
  // server serves every worker. With 1 worker 3 of them sat idle.
  workers: process.env.CI ? 4 : undefined,
  // On CI, failures also become annotations on the check, readable without
  // the report artifact. The blob is what the shards' reports merge from.
  reporter: process.env.CI ? [["github"], ["blob"]] : "html",
  use: {
    baseURL: `http://localhost:${vitePort}`,
    trace: "on-first-retry",
  },
  projects: [
    {
      name: "chromium",
      use: { ...devices["Desktop Chrome"] },
    },
  ],
  webServer: {
    command: `bunx vite dev --mode test --port ${vitePort}`,
    url: `http://localhost:${vitePort}`,
    reuseExistingServer:
      process.env.PLAYWRIGHT_REUSE_EXISTING_SERVER === "true",
    timeout: 30000,
  },
});
