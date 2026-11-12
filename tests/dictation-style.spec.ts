import { expect, test, type Page } from "@playwright/test";

const CLEANUP_OFF = "Styles apply only when cleanup is on.";

/** The setting's row: its title, the (i) tooltip and the dropdown. */
const styleRow = (page: Page, title = "Dictation style") =>
  page
    .locator("div")
    .filter({ has: page.getByRole("heading", { name: title, exact: true }) })
    .filter({ has: page.locator('[aria-haspopup="listbox"]') })
    .last();
const picker = (page: Page) =>
  styleRow(page).locator('[aria-haspopup="listbox"]');

const openTranscription = async (page: Page, query = "") => {
  await page.goto(`/tests/fixtures/settings-pages.html?${query}`);
  await page
    .getByRole("navigation")
    .getByRole("button", { name: "Transcription", exact: true })
    .click();
};

const choose = async (page: Page, name: string) => {
  await picker(page).click();
  await page.getByRole("option", { name, exact: true }).click();
};

const styleWrites = (page: Page) =>
  page.evaluate(() =>
    window.settingsPages.calls
      .filter((call) => call.command === "change_dictation_style_setting")
      .map((call) => call.args),
  );

test.describe("the dictation style in Settings", () => {
  test("is a dropdown row under AI cleanup, explained in its tooltip", async ({
    page,
  }) => {
    await openTranscription(page);
    await expect(picker(page)).toHaveText("Normal");
    await expect(picker(page)).toBeEnabled();

    const cleanup = await page
      .getByRole("checkbox", { name: "Cleanup and formatting" })
      .boundingBox();
    const row = await styleRow(page).boundingBox();
    expect(row!.y, "below AI cleanup").toBeGreaterThan(cleanup!.y);

    await expect(
      styleRow(page).getByRole("button", {
        name: "Formal: full punctuation and written wording. Normal: lighter punctuation, keeps your words. Casual: relaxed, like a chat message.",
        exact: true,
      }),
    ).toBeVisible();

    await picker(page).click();
    await expect(page.getByRole("option")).toHaveText([
      "Formal",
      "Normal",
      "Casual",
    ]);
    expect(
      await page.evaluate(() => window.settingsPages.unhandledCommands),
    ).toEqual([]);
  });

  test("selecting a style saves it", async ({ page }) => {
    await openTranscription(page);
    await choose(page, "Casual");
    await expect(picker(page)).toHaveText("Casual");
    await choose(page, "Formal");
    await expect(picker(page)).toHaveText("Formal");
    expect(await styleWrites(page)).toEqual([
      { style: "casual" },
      { style: "formal" },
    ]);
  });

  test("a refused change says so and keeps the stored style", async ({
    page,
  }) => {
    await openTranscription(page, "style=formal&styleRefused=1");
    await choose(page, "Casual");
    await expect(
      page.getByText("Couldn't change the dictation style."),
    ).toBeVisible();
    await expect(picker(page)).toHaveText("Formal");
  });

  for (const [why, query] of [
    ["with AI cleanup off", "cleanup=off"],
    ["without a ChatGPT account", "account=none"],
  ]) {
    test(`${why}, the dropdown waits for AI cleanup`, async ({ page }) => {
      await openTranscription(page, query);
      await expect(picker(page)).toBeDisabled();
      await expect(
        styleRow(page).getByRole("button", {
          name: new RegExp(`^${CLEANUP_OFF}`),
        }),
      ).toBeVisible();
      await expect(styleRow(page).getByText(CLEANUP_OFF)).toHaveCount(0);
    });
  }

  test("the French row names the styles", async ({ page }) => {
    await openTranscription(page, "lang=fr");
    await styleRow(page, "Style de dictée")
      .locator('[aria-haspopup="listbox"]')
      .click();
    await expect(page.getByRole("option")).toHaveText([
      "Formel",
      "Normal",
      "Détendu",
    ]);
  });
});

const shortToggle = (page: Page) =>
  page.getByRole("checkbox", { name: "Also clean up short dictations" });

test.describe("cleaning up short dictations", () => {
  test("is off by default, under AI cleanup, and saves when turned on", async ({
    page,
  }) => {
    await openTranscription(page);
    await expect(shortToggle(page)).not.toBeChecked();
    await expect(shortToggle(page)).toBeEnabled();
    const cleanup = await page
      .getByRole("checkbox", { name: "Cleanup and formatting" })
      .boundingBox();
    const toggle = await shortToggle(page).boundingBox();
    expect(toggle!.y, "below AI cleanup").toBeGreaterThan(cleanup!.y);
    await expect(
      page.getByRole("button", {
        name: "Dictations under 20 words are pasted right away, without AI cleanup.",
        exact: true,
      }),
    ).toBeVisible();

    await shortToggle(page).check({ force: true });
    await expect(shortToggle(page)).toBeChecked();
    expect(
      await page.evaluate(() =>
        window.settingsPages.calls
          .filter(
            (call) =>
              call.command === "change_cleanup_short_dictations_setting",
          )
          .map((call) => call.args),
      ),
    ).toEqual([{ enabled: true }]);
  });

  test("waits for AI cleanup", async ({ page }) => {
    await openTranscription(page, "cleanup=off");
    await expect(shortToggle(page)).toBeDisabled();
  });

  const outputDir = process.env.MURMUR_SCREENSHOT_DIR;
  for (const theme of ["light", "dark"] as const) {
    test(`screenshot (${theme})`, async ({ page }) => {
      test.skip(!outputDir, "Set MURMUR_SCREENSHOT_DIR to capture screenshots");
      await page.emulateMedia({ colorScheme: theme });
      await openTranscription(page, `theme=${theme}`);
      await shortToggle(page).scrollIntoViewIfNeeded();
      await page.waitForTimeout(500);
      await page.screenshot({
        path: `${outputDir}/ai-cleanup-section-${theme}.png`,
      });
    });
  }
});

const kindPicker = (page: Page, category: string) =>
  page.getByTestId(`style-${category}`).locator('[aria-haspopup="listbox"]');

const kindWrites = (page: Page) =>
  page.evaluate(() =>
    window.settingsPages.calls
      .filter((call) => call.command === "change_category_styles_setting")
      .map((call) => call.args),
  );

test("each kind of app follows the general style until it gets its own", async ({
  page,
}) => {
  await openTranscription(page);
  for (const category of [
    "ai_prompts",
    "personal_messages",
    "work_messages",
    "emails",
    "documents",
    "coding",
  ]) {
    await expect(kindPicker(page, category)).toHaveText("Same as general");
  }
  await expect(page.getByTestId("style-other")).toHaveCount(0);

  await kindPicker(page, "emails").click();
  await page.getByRole("option", { name: "Formal", exact: true }).click();
  await expect(kindPicker(page, "emails")).toHaveText("Formal");

  await kindPicker(page, "coding").click();
  await page.getByRole("option", { name: "No cleanup", exact: true }).click();
  await expect(kindPicker(page, "coding")).toHaveText("No cleanup");

  // Back to the general style: the kind leaves the setting, it is not saved
  // as a style of its own.
  await kindPicker(page, "emails").click();
  await page
    .getByRole("option", { name: "Same as general", exact: true })
    .click();
  await expect(kindPicker(page, "emails")).toHaveText("Same as general");

  expect(await kindWrites(page)).toEqual([
    { styles: { emails: "formal" } },
    { styles: { emails: "formal", coding: "off" } },
    { styles: { coding: "off" } },
  ]);
});

test("the styles by kind of app are off with cleanup", async ({ page }) => {
  await openTranscription(page, "cleanup=off");
  await expect(kindPicker(page, "emails")).toBeDisabled();
  await page
    .getByTestId("style-emails")
    .getByRole("button", { name: new RegExp(`^${CLEANUP_OFF}`) })
    .hover();
  await expect(page.getByRole("tooltip")).toContainText(CLEANUP_OFF);
});
