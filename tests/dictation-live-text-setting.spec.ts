import { expect, test, type Page } from "@playwright/test";
import { readFileSync } from "node:fs";
import { DEFAULT_WINDOW } from "./windowSizes";

const outputDir = process.env.MURMUR_SCREENSHOT_DIR;
const LOCALES = ["en", "fr", "es", "de", "it", "pt"];

const strings = (locale: string) =>
  JSON.parse(
    readFileSync(
      new URL(
        `../src/i18n/locales/${locale}/translation.json`,
        import.meta.url,
      ),
      "utf8",
    ),
  ) as {
    sidebar: { advanced: string };
    settings: {
      advanced: {
        overlay: {
          position: { title: string };
          liveText: { title: string };
        };
      };
    };
  };

const openAdvanced = async (page: Page, query = "", locale = "en") => {
  await page.goto(`/tests/fixtures/settings-pages.html${query}`);
  await page
    .getByRole("navigation")
    .getByRole("button", {
      name: strings(locale).sidebar.advanced,
      exact: true,
    })
    .click();
};

const liveText = (page: Page) =>
  page.getByRole("checkbox", { name: "Show text while dictating" });

const rowTitles = (page: Page) =>
  page
    .locator("h3[title]")
    .evaluateAll((titles) => titles.map((title) => title.textContent));

test("the text shows while dictating only once the switch is turned on", async ({
  page,
}) => {
  await openAdvanced(page);
  await expect(liveText(page)).not.toBeChecked();

  await liveText(page).check({ force: true });

  await expect(liveText(page)).toBeChecked();
  expect(await page.evaluate(() => window.settingsPages.calls)).toContainEqual({
    command: "change_dictation_live_text_setting",
    args: { enabled: true },
  });
  expect(
    await page.evaluate(() => window.settingsPages.unhandledCommands),
  ).toEqual([]);
});

test("the switch follows the overlay rows and says who can read the text", async ({
  page,
}) => {
  await openAdvanced(page);
  await expect(liveText(page)).toBeEnabled();

  const titles = await rowTitles(page);
  expect(titles.indexOf("Show text while dictating")).toBe(
    titles.indexOf("Overlay position") + 1,
  );

  await page.getByRole("button", { name: /^Shows the words next to/ }).click();
  await expect(page.getByRole("tooltip")).toHaveText(
    "Shows the words next to the recording orb as ChatGPT hears them. Anyone who can see your screen can read them.",
  );
});

test("there is no switch while the overlay is off", async ({ page }) => {
  await openAdvanced(page, "?overlay=none&liveText=1");

  await expect(
    page.getByRole("checkbox", { name: "Start Hidden" }),
  ).toBeVisible();
  expect(await rowTitles(page)).not.toContain("Overlay position");
  await expect(liveText(page)).toHaveCount(0);
});

test("with the local engine the switch is off and its bubble starts with the reason", async ({
  page,
}) => {
  await openAdvanced(page, "?engine=local&liveText=1");

  await expect(liveText(page)).toBeDisabled();
  await expect(liveText(page)).not.toBeChecked();

  await page
    .getByRole("button", { name: /^Works with ChatGPT transcription only\./ })
    .click();
  await expect(page.getByRole("tooltip")).toHaveText(
    "Works with ChatGPT transcription only. Shows the words next to the recording orb as ChatGPT hears them. Anyone who can see your screen can read them.",
  );
});

for (const locale of LOCALES) {
  test(`the switch's label fits its row (${locale})`, async ({ page }) => {
    await page.setViewportSize(DEFAULT_WINDOW);
    await openAdvanced(page, `?lang=${locale}`, locale);

    const label = page.locator("h3[title]").filter({
      hasText: strings(locale).settings.advanced.overlay.liveText.title,
    });
    await expect(label).toBeVisible();
    expect(
      await label.evaluate((title) => title.scrollWidth <= title.clientWidth),
    ).toBe(true);
  });
}

for (const theme of ["light", "dark"] as const) {
  test(`the Advanced page with the switch (${theme})`, async ({ page }) => {
    test.skip(!outputDir, "Set MURMUR_SCREENSHOT_DIR to capture screenshots");
    await page.emulateMedia({ colorScheme: theme });
    await page.setViewportSize(DEFAULT_WINDOW);
    await openAdvanced(page, `?theme=${theme}&liveText=1`);
    await expect(liveText(page)).toBeChecked();
    // The sidebar's selection slides over to Advanced.
    await page.waitForTimeout(500);
    await page.screenshot({
      path: `${outputDir}/live-text-setting-${theme}.png`,
    });
  });
}
