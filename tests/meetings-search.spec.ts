import { expect, test, type Page } from "@playwright/test";

const SHOTS = "test-results/shots";

const openMeetings = async (page: Page, theme: "light" | "dark" = "light") => {
  await page.emulateMedia({ colorScheme: theme });
  await page.goto(
    `/tests/fixtures/settings-pages.html?meetings=1&theme=${theme}`,
  );
  await page
    .getByRole("navigation")
    .getByRole("button", { name: "Meetings", exact: true })
    .click();
  await expect(page.getByRole("listitem")).toHaveCount(3);
};

const searchField = (page: Page) =>
  page.getByRole("textbox", { name: "Search meetings" });

test("a search shows the matching meetings with the words marked", async ({
  page,
}) => {
  await openMeetings(page);
  await searchField(page).fill("produit");

  const hits = page.getByRole("listitem");
  await expect(hits).toHaveCount(1);
  await expect(hits.first()).toContainText("Point hebdo produit");
  await expect(hits.first()).toContainText("1:12:05 · Ready");
  await expect(hits.first()).toContainText("on commence par le point produit");
  await expect(hits.first().locator("mark")).toHaveText(["produit", "produit"]);
  await expect(page.getByText("09:41 · Interrupted")).toHaveCount(0);
});

test("a search without results says so", async ({ page }) => {
  await openMeetings(page);
  await searchField(page).fill("zzzz");
  await expect(page.getByText("No results for “zzzz”")).toBeVisible();
  await expect(page.getByRole("listitem")).toHaveCount(0);
});

test("clearing the search shows every meeting again", async ({ page }) => {
  await openMeetings(page);
  await searchField(page).fill("produit");
  await expect(page.getByRole("listitem")).toHaveCount(1);

  await page.getByRole("button", { name: "Clear search" }).click();
  await expect(searchField(page)).toHaveValue("");
  await expect(page.getByRole("listitem")).toHaveCount(3);

  await searchField(page).fill("produit");
  await expect(page.getByRole("listitem")).toHaveCount(1);
  await searchField(page).press("Escape");
  await expect(searchField(page)).toHaveValue("");
  await expect(page.getByRole("listitem")).toHaveCount(3);
});

test("a single letter does not search", async ({ page }) => {
  await openMeetings(page);
  await searchField(page).fill("p");
  await page.waitForTimeout(400);
  await expect(page.getByRole("listitem")).toHaveCount(3);
});

test("a hit opens its meeting on the matching turn, with the transcript search filled in", async ({
  page,
}) => {
  await openMeetings(page);
  await searchField(page).fill("paiement");
  await page.getByRole("button", { name: /Point hebdo produit/ }).click();

  const transcriptTab = page.getByRole("tab", { name: "Transcript" });
  await expect(transcriptTab).toHaveAttribute("aria-selected", "true");
  await expect(
    page.getByRole("textbox", { name: "Search transcript" }),
  ).toHaveValue("paiement");
  const current = page.locator("mark[data-current]");
  await expect(current).toHaveText("paiement");
  await expect(current).toBeInViewport();

  await page.getByRole("button", { name: "All meetings" }).click();
  await expect(searchField(page)).toHaveValue("paiement");
  await expect(page.getByRole("listitem")).toHaveCount(1);
});

test("a hit on the title alone opens the meeting as usual", async ({
  page,
}) => {
  await openMeetings(page);
  await searchField(page).fill("hebdo");
  await page.getByRole("button", { name: /Point hebdo produit/ }).click();

  const summaryTab = page.getByRole("tab", { name: "Summary" });
  await expect(summaryTab).toHaveAttribute("aria-selected", "true");
  await page.getByRole("tab", { name: "Transcript" }).click();
  await expect(
    page.getByRole("textbox", { name: "Search transcript" }),
  ).toHaveCount(0);
});

for (const theme of ["light", "dark"] as const) {
  const suffix = theme === "dark" ? "-dark" : "";
  test(`screenshots of the meeting search (${theme})`, async ({ page }) => {
    await openMeetings(page, theme);
    await searchField(page).fill("produit");
    await expect(page.getByRole("listitem")).toHaveCount(1);
    await page.screenshot({ path: `${SHOTS}/meetings-search${suffix}.png` });

    await searchField(page).fill("zzzz");
    await expect(page.getByText("No results for “zzzz”")).toBeVisible();
    await page.screenshot({
      path: `${SHOTS}/meetings-search-empty${suffix}.png`,
    });
  });
}
