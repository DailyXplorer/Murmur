import { expect, test, type Locator, type Page } from "@playwright/test";
import { DEFAULT_WINDOW } from "./windowSizes";

const ACCENTS = ["pink", "blue", "green", "yellow", "orange", "red"];
const THEMES = ["light", "dark"] as const;

test.use({ viewport: DEFAULT_WINDOW });

const open = async (page: Page, query: string, theme: string) => {
  await page.emulateMedia({ colorScheme: theme as "light" | "dark" });
  await page.goto(
    `/tests/fixtures/settings-pages.html?theme=${theme}&${query}`,
  );
};

const footer = (page: Page) =>
  page
    .locator("div.border-t", {
      has: page.getByText("v1.3.7", { exact: true }),
    })
    .last();

const colours = (element: Locator) =>
  element.evaluate((node) => {
    const style = getComputedStyle(node);
    return {
      background: style.backgroundColor,
      color: style.color,
      ring: style.boxShadow,
    };
  });

for (const theme of THEMES) {
  for (const accent of ACCENTS) {
    test(`the update shows as an accent pill (${accent}, ${theme})`, async ({
      page,
    }) => {
      await open(page, `update=1&accent=${accent}`, theme);
      const pill = page.getByRole("button", { name: "Update available" });
      await expect(pill).toBeVisible();
      const selected = page
        .getByRole("navigation")
        .locator('button[aria-current="page"]');

      await expect
        .poll(async () => {
          const [ours, theirs] = await Promise.all([
            colours(pill),
            colours(selected),
          ]);
          return [ours.background, ours.color].join(" on ") ===
            [theirs.background, theirs.color].join(" on ")
            ? "same as the selected sidebar item"
            : `${ours.color} on ${ours.background}, sidebar ${theirs.color} on ${theirs.background}`;
        })
        .toBe("same as the selected sidebar item");
      expect(
        await page.evaluate(() => document.documentElement.dataset.accentColor),
      ).toBe(accent);

      await pill.focus();
      expect((await colours(pill)).ring).not.toBe("none");
    });
  }

  test(`the footer keeps its height when an update is found (${theme})`, async ({
    page,
  }) => {
    await open(page, "", theme);
    await expect(
      page.getByRole("button", { name: "Check for updates" }),
    ).toBeVisible();
    const without = await footer(page).boundingBox();

    await open(page, "update=1", theme);
    await expect(
      page.getByRole("button", { name: "Update available" }),
    ).toBeVisible();
    const withUpdate = await footer(page).boundingBox();

    expect(withUpdate?.height).toBe(without?.height);
    expect(withUpdate?.y).toBe(without?.y);
  });

  test(`installing the update shows its progress (${theme})`, async ({
    page,
  }) => {
    await open(page, "update=1", theme);
    await page.getByRole("button", { name: "Update available" }).click();
    await expect(
      footer(page).getByText(/Downloading\.\.\.\s+42%/),
    ).toBeVisible();
    await expect(
      page.getByRole("button", { name: "Update available" }),
    ).toHaveCount(0);
    expect(
      await page.evaluate(() => window.settingsPages.unhandledCommands),
    ).toEqual([]);
  });
}
