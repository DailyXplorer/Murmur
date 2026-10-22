import { expect, test } from "@playwright/test";

/** Hindi and Nepali need a Devanagari font, which every Mac has. Without one
 * the browser draws a box for each code point, the text runs about twice as
 * wide as on a Mac, and the layout checks in those locales measure the boxes. */
test("Devanagari text renders with a real font", async ({ page }) => {
  await page.goto("/tests/fixtures/settings-pages.html?lang=hi");
  const { conjunct, letters } = await page.evaluate(() => {
    const width = (text: string) => {
      const span = document.createElement("span");
      span.textContent = text;
      document.body.append(span);
      const { width } = span.getBoundingClientRect();
      span.remove();
      return width;
    };
    return { conjunct: width("क्ष"), letters: width("क") + width("ष") };
  });
  expect(
    conjunct,
    "क्ष should join into one glyph; install a Devanagari font (fonts-lohit-deva on Ubuntu)",
  ).toBeLessThan(letters);
});
