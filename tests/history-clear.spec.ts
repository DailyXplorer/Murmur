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

test("clearing the history asks first, red confirm, says starred and meetings are kept", async ({
  page,
}) => {
  await openHistory(page);

  await page.getByRole("button", { name: "Clear all history…" }).click();
  const dialog = page.getByRole("dialog", { name: "Clear all history?" });
  await expect(dialog).toContainText("except starred ones");
  await expect(dialog).toContainText("Meetings are not affected");

  const confirmColour = await dialog
    .getByRole("button", { name: "Clear history" })
    .evaluate((button) => getComputedStyle(button).backgroundColor);
  const errorColour = await page.evaluate(() => {
    const probe = document.createElement("span");
    probe.style.color = "var(--color-error)";
    document.body.append(probe);
    const colour = getComputedStyle(probe).color;
    probe.remove();
    return colour;
  });
  expect(confirmColour).toBe(errorColour);
  const cancelColour = await dialog
    .getByRole("button", { name: "Cancel" })
    .evaluate((button) => getComputedStyle(button).backgroundColor);
  expect(cancelColour).not.toBe(errorColour);

  await dialog.getByRole("button", { name: "Cancel" }).click();
  await expect(dialog).toHaveCount(0);
  expect(await clearCalls(page)).toBe(0);

  await page.getByRole("button", { name: "Clear all history…" }).click();
  const confirm = page
    .getByRole("dialog", { name: "Clear all history?" })
    .getByRole("button", { name: "Clear history" });
  await confirm.click();
  await expect(dialog).toHaveCount(0);
  expect(await clearCalls(page)).toBe(1);
});
