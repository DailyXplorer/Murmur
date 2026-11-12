import { expect, test } from "@playwright/test";
import { DEFAULT_WINDOW } from "./windowSizes";

const LOCALES = ["en", "fr", "es", "de", "it", "pt"];
const INSIGHTS = 4;

// A row title or a menu's current value that does not fit ends in an
// ellipsis. On Insights the category names do too.
const cutShort = (categories: boolean) =>
  Array.from(
    document.querySelectorAll<HTMLElement>(
      `h3[title], button[aria-haspopup="listbox"] .truncate${categories ? ", li .truncate" : ""}`,
    ),
  )
    .filter((element) => element.scrollWidth > element.clientWidth)
    .map((element) => element.textContent);

for (const locale of LOCALES) {
  test(`no settings label is cut short at the default window size (${locale})`, async ({
    page,
  }) => {
    await page.setViewportSize(DEFAULT_WINDOW);
    await page.goto(`/tests/fixtures/settings-pages.html?lang=${locale}`);
    const sections = page.getByRole("navigation").getByRole("button");
    await expect(sections.first()).toBeVisible();
    const count = await sections.count();
    expect(count).toBeGreaterThan(INSIGHTS);

    for (let index = 0; index < count; index += 1) {
      const section = sections.nth(index);
      await section.click();
      await expect(page.getByRole("heading", { level: 1 })).toHaveText(
        (await section.innerText()).trim(),
      );
      await page.waitForLoadState("networkidle");
      if (index === INSIGHTS) {
        await expect(page.locator("li .truncate").first()).toBeVisible();
      }
      expect(
        await page.evaluate(cutShort, index === INSIGHTS),
        `section ${index}`,
      ).toEqual([]);
    }
  });
}
