import { expect, test } from "@playwright/test";

const fixturePath = "/tests/fixtures/settings-write-contract.html";

test.beforeEach(async ({ page }) => {
  await page.goto(fixturePath);
  await page.waitForFunction(() => Boolean(window.settingsWriteContract));
  await page.evaluate(() => window.settingsWriteContract.reset());
});

test("returns true only after the backend accepts a setting", async ({
  page,
}) => {
  await expect(
    page.evaluate(() => window.settingsWriteContract.updateTheme("dark")),
  ).resolves.toBe(true);
});

test("rolls back only the failed key after a structured backend error", async ({
  page,
}) => {
  await expect(
    page.evaluate(() => window.settingsWriteContract.runRollbackProbe()),
  ).resolves.toEqual({
    historyLimit: 200,
    result: false,
    theme: "system",
  });
});

test("serializes same-key writes and keeps the latest accepted value", async ({
  page,
}) => {
  await expect(
    page.evaluate(() => window.settingsWriteContract.runSameKeyOrderingProbe()),
  ).resolves.toEqual({
    callsBeforeRelease: ["change_theme_setting"],
    finalTheme: "light",
    firstResult: false,
    isUpdating: false,
    secondResult: true,
  });
});

test("refetches when a settings write overlaps a refresh", async ({ page }) => {
  await expect(
    page.evaluate(() => window.settingsWriteContract.runRefreshRaceProbe()),
  ).resolves.toEqual({
    refreshCalls: 2,
    theme: "light",
    updateResult: true,
  });
});

test("ignores an older refresh that resolves after a newer one", async ({
  page,
}) => {
  await expect(
    page.evaluate(() =>
      window.settingsWriteContract.runConcurrentRefreshProbe(),
    ),
  ).resolves.toEqual({
    refreshCalls: 2,
    theme: "light",
  });
});

test("does not enable auto-submit when persisting its key fails", async ({
  page,
}) => {
  await page.evaluate(() =>
    window.settingsWriteContract.failCommand("change_auto_submit_key_setting"),
  );

  const autoSubmit = page.getByTestId("auto-submit");
  await autoSubmit.getByRole("button", { name: "Off" }).click();
  await page.getByRole("option", { name: "Cmd+Enter" }).click();

  await expect
    .poll(() => page.evaluate(() => window.settingsWriteContract.calls()))
    .toEqual(["change_auto_submit_key_setting"]);
});

test("does not apply appearance side effects before persistence succeeds", async ({
  page,
}) => {
  await page.evaluate(() => {
    window.settingsWriteContract.failCommand("change_theme_setting");
    window.settingsWriteContract.failCommand("change_app_language_setting");
    window.settingsWriteContract.failCommand("change_accent_color_setting");
  });

  const theme = page.getByTestId("theme");
  await theme.getByRole("button", { name: "System", exact: true }).click();
  await page.getByRole("option", { name: "Dark" }).click();

  const appLanguage = page.getByTestId("app-language");
  await appLanguage
    .getByRole("button", { name: "English (English)", exact: true })
    .click();
  await page.getByRole("option", { name: "Français (French)" }).click();

  await page
    .getByTestId("accent-color")
    .getByRole("radio", { name: "Blue" })
    .click();

  await expect
    .poll(() => page.evaluate(() => window.settingsWriteContract.calls()))
    .toEqual([
      "change_theme_setting",
      "change_app_language_setting",
      "change_accent_color_setting",
    ]);

  await expect
    .poll(() =>
      page.evaluate(() => window.settingsWriteContract.currentAppearance()),
    )
    .toMatchObject({
      accentColor: "pink",
      appLanguage: "en",
      renderedLanguage: "en",
      theme: "system",
    });

  await expect(
    page.evaluate(() =>
      window.settingsWriteContract
        .currentAppearance()
        .observedAccentColors.includes("blue"),
    ),
  ).resolves.toBe(false);
  await expect(
    page.evaluate(() => document.documentElement.dataset.theme),
  ).resolves.toBeUndefined();
});
