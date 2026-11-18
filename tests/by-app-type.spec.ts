import { expect, test, type Page } from "@playwright/test";
import { MIN_WINDOW } from "./windowSizes";

const openTranscription = async (page: Page, query = "") => {
  await page.goto(`/tests/fixtures/settings-pages.html?${query}`);
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
