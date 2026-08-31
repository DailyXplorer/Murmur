import { defineConfig, devices } from "@playwright/test";

const vitePort = process.env.PLAYWRIGHT_VITE_PORT ?? "1422";

export default defineConfig({
  testDir: "./tests",
  fullyParallel: true,
  forbidOnly: !!process.env.CI,
  retries: process.env.CI ? 2 : 0,
  workers: process.env.CI ? 1 : undefined,
  reporter: "html",
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
