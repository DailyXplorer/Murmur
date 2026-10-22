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
        styleRow(page).getByText(CLEANUP_OFF, { exact: true }),
      ).toBeVisible();
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
