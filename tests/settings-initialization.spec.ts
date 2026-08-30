import { expect, test } from "@playwright/test";

const initializationCounts = (page: import("@playwright/test").Page) =>
  page.evaluate(() => window.settingsInitialization);

test("shares settings initialization during Strict Mode mounts", async ({
  page,
}) => {
  await page.goto("/tests/fixtures/settings-initialization.html");

  await expect
    .poll(() => initializationCounts(page))
    .toEqual({
      customSounds: 1,
      defaultSettings: 1,
      listeners: 1,
      settings: 1,
      unhandledRejections: 0,
    });
});

test("handles settings listener registration failures", async ({ page }) => {
  await page.goto(
    "/tests/fixtures/settings-initialization.html?listener-error",
  );

  await expect
    .poll(() => initializationCounts(page))
    .toEqual({
      customSounds: 1,
      defaultSettings: 1,
      listeners: 1,
      settings: 1,
      unhandledRejections: 0,
    });
});
