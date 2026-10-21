import { expect, test, type Page } from "@playwright/test";

const limitField = (page: Page) => page.getByRole("spinbutton");

const limitWrites = (page: Page) =>
  page.evaluate(() => window.historyLimit.limitWrites());

test.beforeEach(async ({ page }) => {
  await page.goto("/tests/fixtures/history-limit.html");
  await expect(limitField(page)).toHaveValue("50");
});

test("typing a new limit saves only the finished number", async ({ page }) => {
  await limitField(page).selectText();
  await page.keyboard.type("100");
  await expect(limitField(page)).toHaveValue("100");
  expect(await limitWrites(page)).toEqual([]);

  await page.keyboard.press("Enter");

  await expect.poll(() => limitWrites(page)).toEqual([100]);
});

test("editing the limit commits when the field loses focus", async ({
  page,
}) => {
  await limitField(page).press("End");
  await page.keyboard.press("Backspace");
  await page.keyboard.type("5");
  expect(await limitWrites(page)).toEqual([]);

  await page.getByRole("button", { name: "Elsewhere" }).click();

  await expect.poll(() => limitWrites(page)).toEqual([55]);
});

test("a limit below one is saved as one", async ({ page }) => {
  await limitField(page).fill("0");
  await page.keyboard.press("Enter");

  await expect(limitField(page)).toHaveValue("1");
  await expect.poll(() => limitWrites(page)).toEqual([1]);
});

test("an emptied field restores the saved limit", async ({ page }) => {
  await limitField(page).fill("");
  await page.keyboard.press("Enter");

  await expect(limitField(page)).toHaveValue("50");
  expect(await limitWrites(page)).toEqual([]);
});

test("auto-delete says it deletes the transcriptions, not only the audio", async ({
  page,
}) => {
  await expect(
    page.getByText("Auto-delete history", { exact: true }),
  ).toBeVisible();
  await expect(
    page.getByText(
      "Deletes old transcriptions and their audio. Saved entries are kept.",
      { exact: true },
    ),
  ).toBeVisible();
});

test("Escape discards the edit", async ({ page }) => {
  await limitField(page).fill("7");
  await page.keyboard.press("Escape");

  await expect(limitField(page)).toHaveValue("50");
  expect(await limitWrites(page)).toEqual([]);
});
