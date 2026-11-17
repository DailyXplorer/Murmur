import { expect, test, type Page } from "@playwright/test";

const KINDS = [
  "ai_prompts",
  "personal_messages",
  "work_messages",
  "emails",
  "documents",
  "coding",
];

const openTranscription = async (page: Page, query = "") => {
  await page.goto(`/tests/fixtures/settings-pages.html?${query}`);
  await page
    .getByRole("navigation")
    .getByRole("button", { name: "Transcription", exact: true })
    .click();
};

const openByApp = (page: Page, button = "Customize…") =>
  page.getByRole("button", { name: button, exact: true }).click();

const kindLanguage = (page: Page, category: string) =>
  page.getByTestId(`language-${category}`).locator('[aria-haspopup="listbox"]');

const chooseLanguage = async (page: Page, category: string, name: string) => {
  await kindLanguage(page, category).click();
  await page.getByRole("option", { name, exact: true }).click();
};

const languageWrites = (page: Page) =>
  page.evaluate(() =>
    window.settingsPages.calls
      .filter((call) => call.command === "change_category_languages_setting")
      .map((call) => call.args),
  );

test("each kind of app dictates in the general language until it gets its own", async ({
  page,
}) => {
  await openTranscription(page);
  await openByApp(page);
  await expect(
    page.getByRole("dialog", { name: "Style and language by app type" }),
  ).toBeVisible();
  for (const category of KINDS) {
    await expect(kindLanguage(page, category)).toHaveText("Same as general");
  }
  await expect(page.getByTestId("language-other")).toHaveCount(0);

  await chooseLanguage(page, "coding", "English");
  await expect(kindLanguage(page, "coding")).toHaveText("English");

  // A kind can detect the language while the general setting names one.
  await chooseLanguage(page, "ai_prompts", "Auto");
  await expect(kindLanguage(page, "ai_prompts")).toHaveText("Auto");

  // Back to the general language: the kind leaves the setting, it is not
  // saved as a language of its own.
  await chooseLanguage(page, "coding", "Same as general");
  await expect(kindLanguage(page, "coding")).toHaveText("Same as general");

  expect(await languageWrites(page)).toEqual([
    { languages: { coding: "en" } },
    { languages: { coding: "en", ai_prompts: "auto" } },
    { languages: { ai_prompts: "auto" } },
  ]);
  expect(
    await page.evaluate(() => window.settingsPages.unhandledCommands),
  ).toEqual([]);
});

test("the list of a kind of app is searched like the general one", async ({
  page,
}) => {
  await openTranscription(page);
  await openByApp(page);
  await kindLanguage(page, "emails").click();
  await page.getByPlaceholder("Search languages...").fill("fren");
  await expect(page.getByRole("option")).toHaveText(["French"]);

  await page.keyboard.press("Enter");
  await expect(kindLanguage(page, "emails")).toHaveText("French");
  await expect(kindLanguage(page, "emails")).toBeFocused();
  expect(await languageWrites(page)).toEqual([{ languages: { emails: "fr" } }]);
});

test("a language by kind of app needs no AI cleanup, unlike a style", async ({
  page,
}) => {
  await openTranscription(page, "cleanup=off");
  await openByApp(page);
  await expect(
    page.getByTestId("style-emails").locator('[aria-haspopup="listbox"]'),
  ).toBeDisabled();
  await expect(kindLanguage(page, "emails")).toBeEnabled();
});

test("the French rows name the general setting and the languages", async ({
  page,
}) => {
  await openTranscription(page, "lang=fr");
  await openByApp(page, "Personnaliser…");
  await expect(
    page.getByRole("dialog", {
      name: "Style et langue par type d’application",
    }),
  ).toBeVisible();
  await expect(kindLanguage(page, "coding")).toHaveText("Réglage général");
  await kindLanguage(page, "coding").click();
  await page.getByRole("option", { name: "Anglais", exact: true }).click();
  await expect(kindLanguage(page, "coding")).toHaveText("Anglais");
});

test("a slow save does not take the focus back from where the user went", async ({
  page,
}) => {
  await openTranscription(page, "languageSave=slow");
  await openByApp(page);
  await kindLanguage(page, "emails").click();
  await page.getByRole("option", { name: "French", exact: true }).click();

  // While the save runs, the user moves on to another control.
  const style = page
    .getByTestId("style-emails")
    .locator('[aria-haspopup="listbox"]');
  await style.focus();
  await expect(kindLanguage(page, "emails")).toHaveText("French");
  await expect(kindLanguage(page, "emails")).toBeEnabled();

  await expect(style).toBeFocused();
});
