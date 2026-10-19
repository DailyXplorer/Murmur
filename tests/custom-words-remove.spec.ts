import { expect, test } from "@playwright/test";

test("a double click on a custom word removes only that word", async ({
  page,
}) => {
  await page.goto("/tests/fixtures/settings-pages.html");
  await page
    .getByRole("navigation")
    .getByRole("button", { name: "Transcription", exact: true })
    .click();
  const chips = page.getByRole("button", { name: /^Remove / });
  await expect(chips).toHaveCount(3);

  // The next chip slides into this one's place before the second click.
  const box = await chips.first().boundingBox();
  if (!box) throw new Error("not laid out");
  await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
  await page.mouse.down({ clickCount: 1 });
  await page.mouse.up({ clickCount: 1 });
  await expect(chips).toHaveCount(2);
  await expect(chips.first()).toBeEnabled();
  await page.mouse.down({ clickCount: 2 });
  await page.mouse.up({ clickCount: 2 });
  await page.waitForTimeout(100);

  await expect(chips).toHaveCount(2);
  const updates = await page.evaluate(() =>
    window.settingsPages.calls
      .filter(({ command }) => command === "update_custom_words")
      .map(({ args }) => args),
  );
  expect(updates).toEqual([{ words: ["Tauri", "Playwright"] }]);
});
