import { expect, test } from "@playwright/test";

test.describe("General page in another interface language", () => {
  test.beforeEach(async ({ page }) => {
    await page.goto("/tests/fixtures/settings-pages.html?lang=fr");
  });

  test("names the defaults in that language", async ({ page }) => {
    await expect(
      page.getByRole("button", { name: "Automatique", exact: true }),
    ).toBeVisible();
    await expect(
      page.getByRole("button", { name: "Par défaut", exact: true }),
    ).toHaveCount(2);
  });

  test("lists and finds dictation languages by their local name", async ({
    page,
  }) => {
    await page
      .getByRole("button", { name: "Automatique", exact: true })
      .click();
    const search = page.getByPlaceholder("Rechercher des langues...");
    const german = page.getByRole("option", { name: "Allemand", exact: true });

    await search.fill("allem");
    await expect(german).toBeVisible();
    // The English name still finds it.
    await search.fill("German");
    await expect(german).toBeVisible();
  });
});
