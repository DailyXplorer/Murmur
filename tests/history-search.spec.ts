import { expect, test, type Page } from "@playwright/test";

const SHOTS = "test-results/shots";

/** One per dictation shown. */
const entries = (page: Page) =>
  page.getByRole("button", { name: "Delete entry" });

const openHistory = async (page: Page, theme: "light" | "dark" = "light") => {
  await page.emulateMedia({ colorScheme: theme });
  await page.goto(`/tests/fixtures/settings-pages.html?theme=${theme}`);
  await page
    .getByRole("navigation")
    .getByRole("button", { name: "History", exact: true })
    .click();
  await expect(entries(page)).toHaveCount(3);
};

const searchField = (page: Page) =>
  page.getByRole("textbox", { name: "Search dictations" });

test("a search shows the matching dictations with the words marked", async ({
  page,
}) => {
  await openHistory(page);
  await searchField(page).fill("review");

  await expect(entries(page)).toHaveCount(2);
  await expect(page.locator("mark")).toHaveText(["review", "review"]);
  await expect(page.getByText("quarterly report is ready for")).toBeVisible();
  await expect(page.getByText("remember to water the plants")).toHaveCount(0);
});

test("a search result keeps its star and delete", async ({ page }) => {
  await openHistory(page);
  await searchField(page).fill("review");
  await expect(entries(page)).toHaveCount(2);

  await page.getByRole("button", { name: "Save transcription" }).click();
  await expect(
    page.getByRole("button", { name: "Remove from saved" }),
  ).toHaveCount(2);

  await entries(page).first().click();
  await expect(entries(page)).toHaveCount(1);
  await expect(page.getByText("quarterly report is ready for")).toHaveCount(0);
  await expect(page.getByText("design review")).toBeVisible();
});

test("a search without results says so", async ({ page }) => {
  await openHistory(page);
  await searchField(page).fill("zzzz");
  await expect(page.getByText("No results for “zzzz”")).toBeVisible();
  await expect(entries(page)).toHaveCount(0);
});

test("clearing the search shows every dictation again", async ({ page }) => {
  await openHistory(page);
  await searchField(page).fill("review");
  await expect(entries(page)).toHaveCount(2);

  await page.getByRole("button", { name: "Clear search" }).click();
  await expect(searchField(page)).toHaveValue("");
  await expect(entries(page)).toHaveCount(3);
  await expect(page.locator("mark")).toHaveCount(0);

  await searchField(page).fill("review");
  await expect(entries(page)).toHaveCount(2);
  await searchField(page).press("Escape");
  await expect(searchField(page)).toHaveValue("");
  await expect(entries(page)).toHaveCount(3);
});

for (const theme of ["light", "dark"] as const) {
  const suffix = theme === "dark" ? "-dark" : "";
  test(`screenshots of the history search (${theme})`, async ({ page }) => {
    await openHistory(page, theme);
    await searchField(page).fill("review");
    await expect(entries(page)).toHaveCount(2);
    await page.screenshot({ path: `${SHOTS}/history-search${suffix}.png` });

    await searchField(page).fill("zzzz");
    await expect(page.getByText("No results for “zzzz”")).toBeVisible();
    await page.screenshot({
      path: `${SHOTS}/history-search-empty${suffix}.png`,
    });
  });
}
