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

for (const mode of ["Enter", "blur"]) {
  for (const value of ["1e3", "1E3", "1000"]) {
    test(`${mode} commits ${value} as one thousand, never one`, async ({
      page,
    }) => {
      await limitField(page).fill(value);
      if (mode === "Enter") await limitField(page).press("Enter");
      else await page.getByRole("button", { name: "Elsewhere" }).click();

      await expect.poll(() => limitWrites(page)).toEqual([1000]);
      await expect(limitField(page)).toHaveValue("1000");
    });
  }

  for (const value of ["0", "-1", "1001", "1.5", "1e-1"]) {
    test(`${mode} rejects ${value} rather than lowering or clamping the limit`, async ({
      page,
    }) => {
      await limitField(page).fill(value);
      if (mode === "Enter") await limitField(page).press("Enter");
      else await page.getByRole("button", { name: "Elsewhere" }).click();

      expect(await limitWrites(page)).toEqual([]);
      expect(
        await limitField(page).evaluate((input: HTMLInputElement) =>
          input.checkValidity(),
        ),
      ).toBe(false);
      await limitField(page).press("Escape");
      await expect(limitField(page)).toHaveValue("50");
    });
  }
}

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

test("keeping one dictation reads as one in French", async ({ page }) => {
  await page.goto("/tests/fixtures/history-limit.html?lang=fr&keep=1");

  await expect(
    page.getByText("Conserver le plus récent", { exact: true }),
  ).toBeVisible();
  await expect(page.getByText(/Conserver les 1/)).toHaveCount(0);
});

test("Escape discards the edit", async ({ page }) => {
  await limitField(page).fill("7");
  await page.keyboard.press("Escape");

  await expect(limitField(page)).toHaveValue("50");
  expect(await limitWrites(page)).toEqual([]);
});
