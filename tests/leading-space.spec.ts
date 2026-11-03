import { expect, test, type Page } from "@playwright/test";

const LABEL = "Add leading space";
const DESCRIPTION = "Add a space when you dictate right after a word";

const openTranscription = async (
  page: Page,
  { query = "", title = "Transcription" } = {},
) => {
  await page.goto(`/tests/fixtures/settings-pages.html?${query}`);
  await page
    .getByRole("navigation")
    .getByRole("button", { name: title, exact: true })
    .click();
};

const toggle = (page: Page) =>
  page.getByRole("checkbox", { name: LABEL, exact: true });

test("leading space is on above trailing space, with its explanation in a tooltip", async ({
  page,
}) => {
  await openTranscription(page);
  await expect(toggle(page)).toBeChecked();
  await expect(toggle(page)).toBeEnabled();
  await expect(
    page.getByRole("checkbox", { name: "Append trailing space", exact: true }),
  ).not.toBeChecked();

  const headings = await page
    .getByRole("heading", { level: 3 })
    .allTextContents();
  const index = headings.indexOf(LABEL);
  expect(index).toBeGreaterThan(-1);
  expect(headings[index + 1]).toBe("Append trailing space");

  await expect(page.getByText(DESCRIPTION, { exact: true })).toHaveCount(0);
  await page.getByRole("button", { name: DESCRIPTION, exact: true }).hover();
  await expect(page.getByRole("tooltip")).toHaveText(DESCRIPTION);
});

test("the switch saves both choices through the leading-space command", async ({
  page,
}) => {
  await openTranscription(page);
  await toggle(page).uncheck({ force: true });
  await expect(toggle(page)).not.toBeChecked();
  await expect(toggle(page)).toBeEnabled();
  await toggle(page).check({ force: true });
  await expect(toggle(page)).toBeChecked();

  await expect
    .poll(() =>
      page.evaluate(() =>
        window.settingsPages.calls
          .filter((call) => call.command === "change_leading_space_setting")
          .map((call) => call.args),
      ),
    )
    .toEqual([{ enabled: false }, { enabled: true }]);
  expect(
    await page.evaluate(() => window.settingsPages.unhandledCommands),
  ).toEqual([]);
});

test("a saved off choice stays off when transcription settings reopen", async ({
  page,
}) => {
  await openTranscription(page, { query: "leadingSpace=0" });
  await expect(toggle(page)).not.toBeChecked();
  await page
    .getByRole("navigation")
    .getByRole("button", { name: "General", exact: true })
    .click();
  await page
    .getByRole("navigation")
    .getByRole("button", { name: "Transcription", exact: true })
    .click();
  await expect(toggle(page)).not.toBeChecked();
});

test("spacing is available without AI cleanup or a ChatGPT account", async ({
  page,
}) => {
  await openTranscription(page, { query: "cleanup=off&account=none" });
  await expect(toggle(page)).toBeChecked();
  await expect(toggle(page)).toBeEnabled();
});

for (const { language, title, label, description } of [
  {
    language: "en",
    title: "Transcription",
    label: LABEL,
    description: DESCRIPTION,
  },
  {
    language: "fr",
    title: "Transcription",
    label: "Ajouter une espace avant",
    description: "Ajouter une espace quand vous dictez juste après un mot",
  },
  {
    language: "de",
    title: "Transkription",
    label: "Leerzeichen voranstellen",
    description:
      "Ein Leerzeichen hinzufügen, wenn du direkt nach einem Wort diktierst",
  },
  {
    language: "es",
    title: "Transcripción",
    label: "Agregar espacio inicial",
    description:
      "Agregar un espacio cuando dictas justo después de una palabra",
  },
  {
    language: "it",
    title: "Trascrizione",
    label: "Aggiungi spazio iniziale",
    description: "Aggiungi uno spazio quando detti subito dopo una parola",
  },
  {
    language: "pt",
    title: "Transcrição",
    label: "Adicionar espaço inicial",
    description: "Adicionar um espaço quando você dita logo após uma palavra",
  },
]) {
  test(`leading space is translated in ${language}`, async ({ page }) => {
    await openTranscription(page, { query: `lang=${language}`, title });
    await expect(
      page.getByRole("checkbox", { name: label, exact: true }),
    ).toBeChecked();
    await page.getByRole("button", { name: description, exact: true }).hover();
    await expect(page.getByRole("tooltip")).toHaveText(description);
  });
}
