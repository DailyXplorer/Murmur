import { expect, test, type Page } from "@playwright/test";

/** The shortcut field of General whose keys read exactly `keys`. */
const shortcutShowing = (page: Page, keys: string) =>
  page.getByText(keys, { exact: true });

test("the keys that have a name are named in the language of the app", async ({
  page,
}) => {
  await page.goto("/tests/fixtures/settings-pages.html?lang=fr");
  // The space bar has no label to read; Option is what the keyboard prints.
  await expect(shortcutShowing(page, "Option + Espace")).toHaveAttribute(
    "data-control",
    "field",
  );
  await expect(shortcutShowing(page, "Échap")).toHaveAttribute(
    "data-control",
    "field",
  );
  await expect(shortcutShowing(page, "Option + Space")).toHaveCount(0);
});

test("English keeps the names it had", async ({ page }) => {
  await page.goto("/tests/fixtures/settings-pages.html");
  await expect(shortcutShowing(page, "Option + Space")).toHaveAttribute(
    "data-control",
    "field",
  );
  await expect(shortcutShowing(page, "Escape")).toHaveAttribute(
    "data-control",
    "field",
  );
});
