import { expect, test } from "@playwright/test";

test("History that cannot be read says so, and Retry reloads it", async ({
  page,
}) => {
  await page.goto("/tests/fixtures/settings-pages.html?history=fails");
  await page
    .getByRole("navigation")
    .getByRole("button", { name: "History", exact: true })
    .click();

  const alert = page.getByRole("alert");
  await expect(alert).toContainText("Couldn't load your history.");
  await expect(page.getByText(/No transcriptions yet/)).toHaveCount(0);

  // The message starts where the rows' labels do, not centred.
  const message = await alert
    .getByText("Couldn't load your history.")
    .boundingBox();
  const label = await page
    .getByText("History limit", { exact: true })
    .boundingBox();
  expect(Math.abs(message!.x - label!.x)).toBeLessThanOrEqual(1);

  await alert.getByRole("button", { name: "Retry" }).click();
  await expect(alert).toHaveCount(0);
  await expect(
    page.getByRole("button", { name: "Copy transcription to clipboard" }),
  ).not.toHaveCount(0);
});
