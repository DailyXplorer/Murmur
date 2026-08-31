import { expect, test } from "@playwright/test";

test.describe("sound picker accessibility", () => {
  test.beforeEach(async ({ page }) => {
    await page.goto("/tests/fixtures/sound-picker-a11y.html");
  });

  test("exposes the translated preview action name", async ({ page }) => {
    const previewButton = page.getByRole("button", {
      name: "Aperçu",
      exact: true,
    });

    await expect(previewButton).toBeVisible();
    await expect(previewButton).toHaveAttribute("title", "Aperçu");
  });

  test("groups the picker controls under the translated setting name", async ({
    page,
  }) => {
    const picker = page.getByRole("group", {
      name: "Thème sonore",
      exact: true,
    });

    await expect(picker).toBeVisible();
    await expect(
      picker.getByRole("button", { name: "Marimba", exact: true }),
    ).toBeVisible();
  });
});
