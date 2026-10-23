import { expect, test } from "@playwright/test";

test.describe("settings labels in French", () => {
  test.beforeEach(async ({ page }) => {
    await page.goto("/tests/fixtures/settings-french-labels.html");
  });

  test("the default devices and automatic language are named in French", async ({
    page,
  }) => {
    for (const row of ["microphone", "output-device"]) {
      await expect(
        page
          .getByTestId(row)
          .getByRole("button", { name: "Par défaut", exact: true }),
      ).toBeVisible();
    }
    await expect(
      page
        .getByTestId("language")
        .getByRole("button", { name: "Automatique", exact: true }),
    ).toBeVisible();
  });

  test("auto-delete says it deletes the transcriptions, not only the audio", async ({
    page,
  }) => {
    const retention = page.getByTestId("retention");
    await expect(
      retention.getByText("Suppression automatique de l’historique", {
        exact: true,
      }),
    ).toBeVisible();
    await expect(
      retention.getByText(
        "Supprime les anciennes transcriptions et leur audio. Les favoris sont conservés.",
        { exact: true },
      ),
    ).toBeVisible();
  });

  test("languages are listed and found by their French name", async ({
    page,
  }) => {
    await page
      .getByTestId("language")
      .getByRole("button", { name: "Automatique", exact: true })
      .click();
    const search = page.getByRole("textbox");
    const options = page.getByRole("option");

    await search.fill("allemand");
    await expect(options).toHaveText(["Allemand"]);
    // The English name still finds it.
    await search.fill("german");
    await expect(options).toHaveText(["Allemand"]);
  });
});
