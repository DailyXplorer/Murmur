import { expect, test } from "@playwright/test";

test("contains an initial accessibility bridge failure", async ({ page }) => {
  await page.goto("/tests/fixtures/accessibility-check-failure.html");
  await expect(page.getByTestId("accessibility-fixture")).toBeVisible();
  await expect
    .poll(() =>
      page.evaluate(() =>
        window.accessibilityFailureFixture.permissionChecks(),
      ),
    )
    .toBe(1);

  await page.evaluate(
    () => new Promise((resolve) => window.setTimeout(resolve, 50)),
  );

  await expect(
    page.evaluate(() =>
      window.accessibilityFailureFixture.unhandledRejections(),
    ),
  ).resolves.toEqual([]);
  await expect(
    page.getByRole("button", { name: "Allow Access" }),
  ).toBeVisible();
});
