import { expect, test, type Page } from "@playwright/test";

const EXAMPLES = {
  Formal:
    "I think we can meet tomorrow around 3 p.m., if you are available. Otherwise, Thursday also works.",
  Normal:
    "I think we can meet tomorrow around 3 p.m., if you're free. Otherwise, Thursday works too.",
  Casual:
    "I think we can meet tomorrow around 3 if you're free, otherwise Thursday works too",
};
const STYLES = Object.keys(EXAMPLES) as (keyof typeof EXAMPLES)[];

const style = (page: Page, name: string) =>
  page
    .getByRole("radiogroup", { name: "Dictation style" })
    .getByRole("radio", { name: new RegExp(`^${name}`) });

const openTranscription = async (page: Page, query = "") => {
  await page.goto(`/tests/fixtures/settings-pages.html?${query}`);
  await page
    .getByRole("navigation")
    .getByRole("button", { name: "Transcription", exact: true })
    .click();
};

const styleWrites = (page: Page) =>
  page.evaluate(() =>
    window.settingsPages.calls
      .filter((call) => call.command === "change_dictation_style_setting")
      .map((call) => call.args),
  );

test.describe("the dictation style in Settings", () => {
  test("offers the three styles, each with its example, under AI cleanup", async ({
    page,
  }) => {
    await openTranscription(page);
    const group = page.getByRole("radiogroup", { name: "Dictation style" });
    await expect(group.getByRole("radio")).toHaveCount(3);
    for (const name of STYLES) {
      await expect(style(page, name)).toBeEnabled();
      await expect(
        group.getByText(EXAMPLES[name], { exact: true }),
      ).toBeVisible();
    }
    await expect(style(page, "Normal")).toBeChecked();

    const cleanup = await page
      .getByRole("checkbox", { name: "AI cleanup and formatting" })
      .boundingBox();
    const styles = await group.boundingBox();
    expect(styles!.y, "below AI cleanup").toBeGreaterThan(cleanup!.y);
    expect(
      await page.evaluate(() => window.settingsPages.unhandledCommands),
    ).toEqual([]);
  });

  test("selecting a style saves it", async ({ page }) => {
    await openTranscription(page);
    await style(page, "Casual").check();
    await expect(style(page, "Casual")).toBeChecked();
    await style(page, "Formal").check();
    await expect(style(page, "Formal")).toBeChecked();
    expect(await styleWrites(page)).toEqual([
      { style: "casual" },
      { style: "formal" },
    ]);
  });

  test("a refused change says so and keeps the stored style", async ({
    page,
  }) => {
    await openTranscription(page, "style=formal&styleRefused=1");
    await style(page, "Casual").click();
    await expect(
      page.getByText("Couldn't change the dictation style."),
    ).toBeVisible();
    await expect(style(page, "Formal")).toBeChecked();
  });

  test("with AI cleanup off, the styles wait for it", async ({ page }) => {
    await openTranscription(page, "cleanup=off");
    for (const name of STYLES) await expect(style(page, name)).toBeDisabled();
    await expect(
      page.getByText("Styles apply only when AI cleanup is on.", {
        exact: true,
      }),
    ).toBeVisible();
  });

  test("without a ChatGPT account, the styles say why they are off", async ({
    page,
  }) => {
    await openTranscription(page, "account=none");
    for (const name of STYLES) await expect(style(page, name)).toBeDisabled();
    await expect(
      page.getByText("Styles need AI cleanup, which needs a ChatGPT account.", {
        exact: true,
      }),
    ).toBeVisible();
    await expect(
      page.getByText("Styles apply only when AI cleanup is on."),
    ).toHaveCount(0);
  });

  test("the French page names the styles and shows French examples", async ({
    page,
  }) => {
    await page.goto("/tests/fixtures/settings-pages.html?lang=fr");
    await page
      .getByRole("navigation")
      .getByRole("button", { name: "Transcription", exact: true })
      .click();
    const group = page.getByRole("radiogroup", { name: "Style de dictée" });
    for (const name of ["Formel", "Normal", "Détendu"]) {
      await expect(
        group.getByRole("radio", { name: new RegExp(`^${name}`) }),
      ).toBeVisible();
    }
    await expect(
      group.getByText(
        "Je pense qu'on peut se voir demain vers 15h si t'es dispo, sinon jeudi ça marche aussi",
        { exact: true },
      ),
    ).toBeVisible();
  });
});
