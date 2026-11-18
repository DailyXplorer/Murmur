import { expect, test, type Page } from "@playwright/test";
import { readdirSync, readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { DEFAULT_WINDOW, MIN_WINDOW } from "./windowSizes";

const fixture = "/tests/fixtures/settings-pages.html";

const localesDir = fileURLToPath(
  new URL("../src/i18n/locales/", import.meta.url),
);
const LOCALES = readdirSync(localesDir).map((locale) => {
  const { sidebar, settings } = JSON.parse(
    readFileSync(`${localesDir}${locale}/translation.json`, "utf8"),
  ) as {
    sidebar: { transcription: string };
    settings: {
      transcription: {
        byApp: { customize: string };
        style: { cleanupOff: string };
      };
    };
  };
  return {
    locale,
    section: sidebar.transcription,
    customizeLabel: settings.transcription.byApp.customize,
    noteText: settings.transcription.style.cleanupOff,
  };
});

const openTranscription = async (page: Page, query = "") => {
  await page.goto(`${fixture}?${query}`);
  await page
    .getByRole("navigation")
    .getByRole("button", { name: "Transcription", exact: true })
    .click();
};

const customize = (page: Page) =>
  page.getByRole("button", { name: "Customize…", exact: true });
const dialog = (page: Page) =>
  page.getByRole("dialog", { name: "Style and language by app type" });
const picker = (page: Page, cell: string) =>
  page.getByTestId(cell).locator('[aria-haspopup="listbox"]');
const status = (page: Page) =>
  page.locator("[data-live-status]").filter({ hasText: "customized" });
/** The (i) after the dialog's title, there while styles cannot apply. */
const note = (page: Page) =>
  dialog(page).getByRole("button", {
    name: "Styles apply only when cleanup is on.",
    exact: true,
  });
const bubble = (page: Page) => page.getByRole("tooltip");

test("the page holds one row for the six kinds of app, and says how many are customized", async ({
  page,
}) => {
  await openTranscription(page);

  await expect(
    page.getByRole("heading", { name: "By app type", exact: true }),
  ).toBeVisible();
  // The kinds are in the dialog, not on the page.
  await expect(page.getByTestId("style-emails")).toHaveCount(0);
  await expect(page.getByTestId("language-emails")).toHaveCount(0);
  await expect(status(page)).toHaveCount(0);

  await customize(page).click();
  await expect(dialog(page)).toBeVisible();
  await picker(page, "style-emails").click();
  await page.getByRole("option", { name: "Formal", exact: true }).click();
  await picker(page, "language-emails").click();
  await page.getByRole("option", { name: "French", exact: true }).click();
  await picker(page, "language-coding").click();
  await page.getByRole("option", { name: "English", exact: true }).click();

  await dialog(page).getByRole("button", { name: "Close" }).click();
  await expect(dialog(page)).toHaveCount(0);
  // Emails has a style and a language: it counts once.
  await expect(status(page)).toHaveText("2 app types customized");
  await expect(customize(page)).toBeFocused();

  await customize(page).click();
  await picker(page, "language-coding").click();
  await page
    .getByRole("option", { name: "Same as general", exact: true })
    .click();
  await page.keyboard.press("Escape");
  await expect(status(page)).toHaveText("1 app type customized");
});

test("Escape closes an open list before it closes the dialog", async ({
  page,
}) => {
  await openTranscription(page);
  await customize(page).click();
  await picker(page, "style-coding").click();
  await expect(page.getByRole("listbox")).toBeVisible();

  await page.keyboard.press("Escape");
  await expect(page.getByRole("listbox")).toHaveCount(0);
  await expect(dialog(page)).toBeVisible();

  await page.keyboard.press("Escape");
  await expect(dialog(page)).toHaveCount(0);
});

test("Escape closes the note's bubble before it closes the dialog", async ({
  page,
}) => {
  await openTranscription(page, "cleanup=off");
  await customize(page).click();
  await note(page).click();
  await expect(bubble(page)).toBeVisible();

  await page.keyboard.press("Escape");
  await expect(bubble(page)).toHaveCount(0);
  await expect(dialog(page)).toBeVisible();

  await page.keyboard.press("Escape");
  await expect(dialog(page)).toHaveCount(0);
});

test("Tab stays in the dialog while the note's bubble is open", async ({
  page,
}) => {
  await openTranscription(page, "cleanup=off");
  await customize(page).click();
  await expect(dialog(page)).toBeFocused();
  await page.keyboard.press("Tab");
  await expect(note(page)).toBeFocused();
  await expect(bubble(page)).toBeVisible();

  // The note is the first control of the dialog: one step back is its last.
  await page.keyboard.press("Shift+Tab");
  await expect(picker(page, "language-coding")).toBeFocused();
});

for (const size of [DEFAULT_WINDOW, MIN_WINDOW]) {
  test(`the note's bubble is whole and above the dialog in every locale (${size.width}x${size.height})`, async ({
    page,
  }) => {
    await page.setViewportSize(size);
    for (const { locale, section, customizeLabel, noteText } of LOCALES) {
      await page.goto(`${fixture}?cleanup=off&lang=${locale}`);
      await page
        .getByRole("navigation")
        .getByRole("button", { name: section, exact: true })
        .click();
      await page
        .getByRole("button", { name: customizeLabel, exact: true })
        .click();
      await page
        .getByRole("dialog")
        .getByRole("button", { name: noteText, exact: true })
        .hover();
      await expect(bubble(page)).toHaveText(noteText);
      await expect(bubble(page)).toHaveCSS("opacity", "1");

      const seen = await bubble(page).evaluate((element) => {
        const { left, top, right, bottom } = element.getBoundingClientRect();
        const centerX = (left + right) / 2;
        const centerY = (top + bottom) / 2;
        return {
          inWindow:
            left >= 0 &&
            top >= 0 &&
            right <= window.innerWidth &&
            bottom <= window.innerHeight,
          onTop: [
            [centerX, centerY],
            [centerX, top + 2],
            [centerX, bottom - 2],
            [left + 2, centerY],
            [right - 2, centerY],
          ].every(([x, y]) =>
            element.contains(document.elementFromPoint(x, y)),
          ),
          textFits: element.scrollWidth <= element.clientWidth,
        };
      });
      expect(seen, locale).toEqual({
        inWindow: true,
        onTop: true,
        textFits: true,
      });
    }
  });
}

test("the dialog fits the smallest window with every label whole, in French", async ({
  page,
}) => {
  await page.setViewportSize(MIN_WINDOW);
  await openTranscription(page, "lang=fr");
  await page.getByRole("button", { name: "Personnaliser…" }).click();
  const box = page.getByRole("dialog");
  await expect(box).toBeVisible();

  const whole = await box.evaluate((element) => {
    const fits = (node: Element) => node.scrollWidth <= node.clientWidth;
    return (
      [...element.querySelectorAll("span[title]")].every(fits) &&
      [...element.querySelectorAll('[aria-haspopup="listbox"] span')].every(
        fits,
      )
    );
  });
  expect(whole).toBe(true);
  const { x, width } = (await box.boundingBox())!;
  expect(x).toBeGreaterThanOrEqual(0);
  expect(x + width).toBeLessThanOrEqual(MIN_WINDOW.width);
});

const outputDir = process.env.MURMUR_SCREENSHOT_DIR;
for (const theme of ["light", "dark"] as const) {
  test(`screenshot of the note's bubble (${theme})`, async ({ page }) => {
    test.skip(!outputDir, "Set MURMUR_SCREENSHOT_DIR to capture screenshots");
    await page.emulateMedia({ colorScheme: theme });
    await page.setViewportSize(DEFAULT_WINDOW);
    await openTranscription(page, `cleanup=off&theme=${theme}`);
    await customize(page).click();
    await note(page).hover();
    await expect(bubble(page)).toHaveCSS("opacity", "1");
    await page.screenshot({
      path: `${outputDir}/by-app-note-bubble-${theme}.png`,
    });
  });
}
