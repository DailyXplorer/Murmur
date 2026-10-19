import { expect, test } from "@playwright/test";

test("a double click on an entry's trash deletes only that entry", async ({
  page,
}) => {
  await page.goto("/tests/fixtures/settings-pages.html");
  await page
    .getByRole("navigation")
    .getByRole("button", { name: "History", exact: true })
    .click();
  const trash = page.getByRole("button", { name: "Delete entry" });
  await expect(trash).toHaveCount(3);

  // The next entry's trash takes this one's place before the second click.
  const box = await trash.first().boundingBox();
  if (!box) throw new Error("not laid out");
  await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
  await page.mouse.down({ clickCount: 1 });
  await page.mouse.up({ clickCount: 1 });
  await expect(trash).toHaveCount(2);
  await page.mouse.down({ clickCount: 2 });
  await page.mouse.up({ clickCount: 2 });
  await page.waitForTimeout(100);

  await expect(trash).toHaveCount(2);
  const deletes = await page.evaluate(() =>
    window.settingsPages.calls
      .filter(({ command }) => command === "delete_history_entry")
      .map(({ args }) => args),
  );
  expect(deletes).toEqual([{ id: 3 }]);
});
