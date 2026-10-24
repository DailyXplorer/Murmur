import { expect, test } from "@playwright/test";

test("the Debug controls have translated names", async ({ page }) => {
  await page.goto("/tests/fixtures/settings-pages.html");
  await page
    .getByRole("navigation")
    .getByRole("button", { name: "Debug", exact: true })
    .click();

  await expect(
    page.getByRole("button", {
      name: "Reset: Clamshell microphone",
      exact: true,
    }),
  ).toBeVisible();
  const preview = page.getByRole("button", {
    name: "Preview sound theme",
    exact: true,
  });
  await expect(preview).toBeVisible();
  await expect(preview).toHaveAttribute("title", "Preview sound theme");
});
