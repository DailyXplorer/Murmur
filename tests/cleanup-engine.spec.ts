import { expect, test, type Page } from "@playwright/test";

const openTranscription = async (page: Page, apple: string, query = "") => {
  await page.goto(`/tests/fixtures/settings-pages.html?apple=${apple}${query}`);
  await page
    .getByRole("navigation")
    .getByRole("button", { name: "Transcription", exact: true })
    .click();
};

const engineDropdown = (page: Page) =>
  page.locator('[aria-haspopup="listbox"]').filter({
    hasText: /^(Apple Intelligence \(on this Mac\)|Codex \(ChatGPT\))$/,
  });

test.describe("AI cleanup engine", () => {
  test("defaults to Apple Intelligence and shows it is ready", async ({
    page,
  }) => {
    await openTranscription(page, "available");
    await expect(engineDropdown(page)).toHaveText(
      "Apple Intelligence (on this Mac)",
    );
    await expect(page.getByTestId("apple-intelligence-status")).toHaveText(
      "Ready on this Mac",
    );
    expect(
      await page.evaluate(() => window.settingsPages.unhandledCommands),
    ).toEqual([]);
  });

  test("tells the user how to turn Apple Intelligence on", async ({ page }) => {
    await openTranscription(page, "appleIntelligenceNotEnabled");
    await expect(page.getByTestId("apple-intelligence-status")).toHaveText(
      "Turn on Apple Intelligence in System Settings",
    );
  });

  test("names the dictation language Apple Intelligence lacks", async ({
    page,
  }) => {
    await openTranscription(page, "unsupportedLanguage", "&dictation=ru");
    await expect(page.getByTestId("apple-intelligence-status")).toHaveText(
      "Not available in Russian",
    );
  });

  test("names the language in the interface language", async ({ page }) => {
    await openTranscription(
      page,
      "unsupportedLanguage",
      "&dictation=ru&lang=fr",
    );
    await expect(page.getByTestId("apple-intelligence-status")).toHaveText(
      "Non disponible en russe",
    );
  });

  test("an older macOS is told which version it needs", async ({ page }) => {
    await openTranscription(page, "unsupportedOs");
    await expect(page.getByTestId("apple-intelligence-status")).toHaveText(
      "Requires macOS 26 or later",
    );
  });

  test("choosing Codex saves it and hides the Apple status", async ({
    page,
  }) => {
    await openTranscription(page, "modelNotReady");
    await expect(page.getByTestId("apple-intelligence-status")).toHaveText(
      "Still downloading, try again later",
    );
    await engineDropdown(page).click();
    await page
      .getByRole("option", { name: "Codex (ChatGPT)", exact: true })
      .click();

    await expect(engineDropdown(page)).toHaveText("Codex (ChatGPT)");
    await expect(page.getByTestId("apple-intelligence-status")).toHaveCount(0);
    const writes = await page.evaluate(() =>
      window.settingsPages.calls.filter(
        (call) => call.command === "change_cleanup_engine_setting",
      ),
    );
    expect(writes.map((call) => call.args)).toEqual([{ engine: "codex" }]);
  });
});
