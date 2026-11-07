import { expect, test, type Page } from "@playwright/test";

const openHistory = async (page: Page) => {
  await page.goto("/tests/fixtures/settings-pages.html");
  await page
    .getByRole("navigation")
    .getByRole("button", { name: "History", exact: true })
    .click();
  await expect(page.getByRole("button", { name: "Delete entry" })).toHaveCount(
    3,
  );
};

const clearCalls = (page: Page) =>
  page.evaluate(
    () =>
      window.settingsPages.calls.filter(
        ({ command }) => command === "clear_history",
      ).length,
  );

test("clearing the history asks first and says meetings are kept", async ({
  page,
}) => {
  await openHistory(page);

  await page.getByRole("button", { name: "Clear all history…" }).click();
  const dialog = page.getByRole("dialog", { name: "Clear all history?" });
  await expect(dialog).toContainText("saved ones included");
  await expect(dialog).toContainText("Meetings are not affected");

  await dialog.getByRole("button", { name: "Cancel" }).click();
  await expect(dialog).toHaveCount(0);
  expect(await clearCalls(page)).toBe(0);

  await page.getByRole("button", { name: "Clear all history…" }).click();
  await page
    .getByRole("dialog", { name: "Clear all history?" })
    .getByRole("button", { name: "Clear history" })
    .click();
  await expect(dialog).toHaveCount(0);
  expect(await clearCalls(page)).toBe(1);
});
