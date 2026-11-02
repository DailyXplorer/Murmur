import { expect, test } from "@playwright/test";

const ENGLISH = { title: "Application language", option: "English (English)" };
const PORTUGUESE = {
  title: "Idioma da aplicação",
  option: "Português (Portuguese)",
};

test("the interface offers six languages, English first", async ({ page }) => {
  await page.goto("/tests/fixtures/app-language.html?saved=en");
  await page.getByRole("button", { name: ENGLISH.option }).click();

  await expect(page.getByRole("option")).toHaveText([
    "English (English)",
    "Français (French)",
    "Español (Spanish)",
    "Deutsch (German)",
    "Italiano (Italian)",
    "Português (Portuguese)",
  ]);
});

for (const { saved, system, shown } of [
  { saved: "ja", system: "en-US", shown: ENGLISH },
  { saved: "zh-TW", system: "en-US", shown: ENGLISH },
  { saved: "ar", system: "en-US", shown: ENGLISH },
  { saved: "pt-BR", system: "en-US", shown: PORTUGUESE },
  { saved: "", system: "zh-Hant-TW", shown: ENGLISH },
  { saved: "", system: "ru_RU", shown: ENGLISH },
  { saved: "", system: "pt_BR", shown: PORTUGUESE },
]) {
  test(`saved "${saved}" with system "${system}" shows ${shown.option}`, async ({
    page,
  }) => {
    await page.goto(
      `/tests/fixtures/app-language.html?saved=${saved}&system=${system}`,
    );

    await expect(page.getByText(shown.title, { exact: true })).toBeVisible();
    await expect(
      page.getByRole("button", { name: shown.option, exact: true }),
    ).toBeVisible();
  });
}

test("a French setting paints French first and loads only English and French", async ({
  page,
}) => {
  const bundles = new Set<string>();
  page.on("request", (request) => {
    const code = request
      .url()
      .match(/\/locales\/([^/]+)\/translation\.json/)?.[1];
    if (code) bundles.add(code);
  });

  await page.goto("/tests/fixtures/app-language.html?saved=fr");

  await expect(
    page.getByText("Langue de l'application", { exact: true }),
  ).toBeVisible();
  expect(await page.evaluate(() => window.appLanguageFirstPaint)).toContain(
    "Langue de l'application",
  );
  expect(await page.evaluate(() => document.documentElement.lang)).toBe("fr");
  expect([...bundles].sort()).toEqual(["en", "fr"]);
  const outputDir = process.env.MURMUR_SCREENSHOT_DIR;
  if (outputDir) {
    await page.screenshot({ path: `${outputDir}/first-paint-fr.png` });
  }
});
