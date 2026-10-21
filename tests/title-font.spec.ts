import { expect, test, type Locator } from "@playwright/test";

const fixture = "/tests/fixtures/settings-pages.html";

const fontFamily = (element: Locator) =>
  element.evaluate((node) => getComputedStyle(node).fontFamily);

test("section titles use Faculty Glyphic, except in Vietnamese", async ({
  page,
}) => {
  await page.goto(`${fixture}?lang=en`);
  const title = page.getByRole("heading", { level: 2 }).first();
  await expect(title).toBeVisible();
  expect(await fontFamily(title)).toMatch(/^"Faculty Glyphic"/);

  // Faculty Glyphic has no glyph for Vietnamese tones (U+1EA0-1EF9), so a
  // word would switch fonts letter by letter.
  await page.goto(`${fixture}?lang=vi`);
  await expect(title).toBeVisible();
  expect(await fontFamily(title)).toMatch(/^"Instrument Sans Variable"/);
});

test("a Vietnamese meeting title keeps the body font", async ({ page }) => {
  await page.goto(`${fixture}?meetings=1`);
  await page
    .getByRole("navigation")
    .getByRole("button", { name: "Meetings", exact: true })
    .click();
  await page.getByRole("button", { name: /Point hebdo produit/ }).click();
  const title = page.getByLabel("Meeting title");

  expect(await fontFamily(title)).toMatch(/^"Faculty Glyphic"/);
  await title.fill("Họp nhóm hàng tuần");
  expect(await fontFamily(title)).toMatch(/^"Instrument Sans Variable"/);
});
