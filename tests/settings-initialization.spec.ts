import { expect, test } from "@playwright/test";

const initializationCounts = (page: import("@playwright/test").Page) =>
  page.evaluate(() => window.settingsInitialization);

const emitSettingsChanged = (page: import("@playwright/test").Page) =>
  page.evaluate(() => window.settingsInitialization.emitSettingsChanged());

const openFixture = async (
  page: import("@playwright/test").Page,
  query = "",
) => {
  await page.goto(`/tests/fixtures/settings-initialization.html${query}`);
  await expect(page.locator("body")).toHaveAttribute(
    "data-settings-initialization-fixture",
    "true",
  );
};

test("shares settings initialization during Strict Mode mounts", async ({
  page,
}) => {
  await openFixture(page);

  await expect
    .poll(() => initializationCounts(page))
    .toEqual({
      customSounds: 1,
      defaultSettings: 1,
      listeners: 1,
      settings: 1,
      unhandledRejections: 0,
    });

  await emitSettingsChanged(page);

  await expect
    .poll(() => initializationCounts(page))
    .toEqual({
      customSounds: 1,
      defaultSettings: 1,
      listeners: 1,
      settings: 2,
      unhandledRejections: 0,
    });
});

test("handles settings listener registration failures", async ({ page }) => {
  await openFixture(page, "?listener-failures=1");

  await expect
    .poll(() => initializationCounts(page))
    .toEqual({
      customSounds: 1,
      defaultSettings: 1,
      listeners: 2,
      settings: 1,
      unhandledRejections: 0,
    });

  await emitSettingsChanged(page);

  await expect
    .poll(() => initializationCounts(page))
    .toEqual({
      customSounds: 1,
      defaultSettings: 1,
      listeners: 2,
      settings: 2,
      unhandledRejections: 0,
    });
});

test("bounds autonomous retries after repeated listener failures", async ({
  page,
}) => {
  await openFixture(page, "?listener-failures=3");

  await expect
    .poll(() => initializationCounts(page))
    .toEqual({
      customSounds: 1,
      defaultSettings: 1,
      listeners: 3,
      settings: 1,
      unhandledRejections: 0,
    });

  await page.waitForTimeout(1100);

  await expect
    .poll(() => initializationCounts(page))
    .toEqual({
      customSounds: 1,
      defaultSettings: 1,
      listeners: 3,
      settings: 1,
      unhandledRejections: 0,
    });
});

test("chains HMR listener cleanup before replacing a late registration", async ({
  page,
}) => {
  await openFixture(page);

  await expect
    .poll(() =>
      page.evaluate(() => window.settingsInitialization.unhandledRejections),
    )
    .toBe(0);

  await expect(
    page.evaluate(() => window.settingsInitialization.runHmrLifecycleRace()),
  ).resolves.toEqual({
    activeListeners: 1,
    aEvents: 0,
    cEvents: 1,
    listenerRegistrations: 2,
    maximumActiveListeners: 1,
  });

  await expect
    .poll(() =>
      page.evaluate(() => window.settingsInitialization.unhandledRejections),
    )
    .toBe(0);
});
