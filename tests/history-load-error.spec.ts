import { expect, test } from "@playwright/test";

test("History that cannot be read says so, not that it is empty", async ({
  page,
}) => {
  await page.goto("/tests/fixtures/settings-pages.html?history=fails");
  await page
    .getByRole("navigation")
    .getByRole("button", { name: "History", exact: true })
    .click();

  await expect(page.getByText("Couldn't load History.")).toBeVisible();
  await expect(page.getByText(/No transcriptions yet/)).toHaveCount(0);
});
