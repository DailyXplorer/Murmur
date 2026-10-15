import { expect, test, type Page } from "@playwright/test";

const openInsights = async (page: Page, query = "") => {
  await page.setViewportSize({ width: 900, height: 1000 });
  await page.goto(`/tests/fixtures/insights.html${query}`);
  await page.waitForFunction(() => window.insights?.ready === true);
};

test("shows speed, totals, fixes, app usage and streak", async ({ page }) => {
  await openInsights(page);
  const main = page.getByRole("main", { name: "Insights" });

  await expect(main.getByText("142", { exact: true })).toBeVisible();
  await expect(main.getByText("48,210", { exact: true })).toBeVisible();
  await expect(main.getByText("≈ 6.4 × a short story")).toBeVisible();
  await expect(main.getByText("57", { exact: true })).toBeVisible();
  await expect(main.getByText("Words corrected: 12")).toBeVisible();
  await expect(main.getByText("Dictionary fixes: 45")).toBeVisible();

  const rows = main.getByRole("listitem");
  await expect(rows).toHaveText([
    "AI prompts48%96",
    "Coding & terminal27%54",
    "Work messages16%31",
    "Emails6%12",
    "Other4%7",
  ]);
  await expect(main.getByText("Total apps used")).toBeVisible();
  await expect(main.getByText("14", { exact: true })).toBeVisible();

  await expect(main.getByText("12 days")).toBeVisible();
  await expect(main.getByText("31 days")).toBeVisible();
  await expect(main.getByText("May", { exact: true })).toHaveCount(0);
  await expect(main.getByText("Jun", { exact: true })).toBeVisible();
  const cells = main.locator("[data-level]");
  await expect(cells).toHaveCount(19 * 7 + 5);
  await expect(cells.last()).toHaveAttribute(
    "title",
    "Dictations on Oct 15, 2026: 11",
  );

  await expect(page.getByRole("button", { name: "Insights" })).toHaveAttribute(
    "aria-current",
    "page",
  );
});

test("explains the empty state before any dictation", async ({ page }) => {
  await openInsights(page, "?data=empty");
  const main = page.getByRole("main", { name: "Insights" });

  await expect(main.getByText("–", { exact: true })).toBeVisible();
  await expect(main.getByText("Not enough timed dictations yet")).toBeVisible();
  await expect(main.getByText("Start dictating to fill this in")).toBeVisible();
  await expect(
    main.getByText(
      "App categories are recorded for new dictations only. Dictate in any app to see this fill in.",
    ),
  ).toBeVisible();
  await expect(main.getByText("0 days")).toHaveCount(2);
  await expect(main.locator('[data-level="0"]')).toHaveCount(19 * 7 + 5);
});

test("refreshes when the backend reports new insights", async ({ page }) => {
  await openInsights(page, "?data=empty");
  const main = page.getByRole("main", { name: "Insights" });
  await expect(main.getByText("–", { exact: true })).toBeVisible();

  await page.evaluate(() =>
    window.insights.publish({
      total_dictations: 40,
      total_words: 1_250,
      words_per_minute: 120,
      words_corrected: 0,
      dictionary_fixes: 3,
      categories: [{ category: "documents", count: 1 }],
      categorized_dictations: 1,
      apps_used: 1,
      streaks: { current: 1, longest: 1 },
      heatmap: [],
    }),
  );

  await expect(main.getByText("1,250", { exact: true })).toBeVisible();
  await expect(main.getByText("≈ 13 × a typical email")).toBeVisible();
  expect(await page.evaluate(() => window.insights.requests())).toBe(2);
});

for (const theme of ["light", "dark"] as const) {
  test(`renders in ${theme} mode`, async ({ page }) => {
    await openInsights(page, `?theme=${theme}`);
    await expect(page.getByText("48,210", { exact: true })).toBeVisible();
    const background = await page.evaluate(
      () =>
        getComputedStyle(document.body.firstElementChild!.firstElementChild!)
          .backgroundColor,
    );
    expect(background).toBe(
      theme === "light" ? "rgb(251, 251, 251)" : "rgb(44, 43, 41)",
    );
    const dir = process.env.INSIGHTS_SCREENSHOT_DIR;
    if (dir) {
      await page.screenshot({
        path: `${dir}/murmur-insights-${theme}.png`,
        fullPage: true,
      });
    }
  });
}
