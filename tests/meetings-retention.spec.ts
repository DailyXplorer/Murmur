import { expect, test, type Page } from "@playwright/test";
import { DEFAULT_WINDOW } from "./windowSizes";

const fixture = "/tests/fixtures/settings-pages.html";
const outputDir = process.env.MURMUR_SCREENSHOT_DIR;

const openAdvanced = async (page: Page, query = "") => {
  await page.goto(`${fixture}${query}`);
  await page
    .getByRole("navigation")
    .getByRole("button", { name: "Advanced", exact: true })
    .click();
};

const appGroup = (page: Page) =>
  page.locator("section").filter({
    has: page.getByRole("heading", { name: "App", exact: true }),
  });

/** The innermost block holding both the setting's title and its menu. */
const retentionRow = (page: Page) =>
  appGroup(page)
    .locator("div")
    .filter({
      has: page.getByRole("heading", { name: "Delete meeting audio" }),
    })
    .filter({ has: page.locator('[aria-haspopup="listbox"]') })
    .last();

const calls = (page: Page) => page.evaluate(() => window.settingsPages.calls);

test("meeting audio is kept by default, and the space meetings use is shown", async ({
  page,
}) => {
  await openAdvanced(page);
  const row = retentionRow(page);

  await expect(row.getByRole("button", { name: "Never" })).toBeVisible();
  await expect(row).toContainText("Transcripts and summaries are kept.");
  await expect(row).toContainText("Meeting audio uses 1.2 GB.");
  expect(
    await page.evaluate(() => window.settingsPages.unhandledCommands),
  ).toEqual([]);
});

test("choosing a delay saves it", async ({ page }) => {
  await openAdvanced(page);
  const row = retentionRow(page);

  await row.getByRole("button", { name: "Never" }).click();
  await page.getByRole("option", { name: "After 30 days" }).click();

  await expect(
    row.getByRole("button", { name: "After 30 days" }),
  ).toBeVisible();
  expect(await calls(page)).toContainEqual({
    command: "change_meeting_audio_retention_setting",
    args: { retention: "days30" },
  });
});

test("recordings set aside are counted on their own", async ({ page }) => {
  await openAdvanced(page, "?storageBytes=482000000&setAsideBytes=96000000");

  await expect(retentionRow(page)).toContainText(
    "Meeting audio uses 482 MB, plus 96 MB of unlisted recordings, deleted after 30 days.",
  );
});

test("the setting is hidden while meetings are off", async ({ page }) => {
  await openAdvanced(page, "?meetings=0");
  await expect(
    appGroup(page).getByRole("checkbox", { name: "Meetings" }),
  ).not.toBeChecked();

  await expect(page.getByText("Delete meeting audio")).toHaveCount(0);
  expect((await calls(page)).map(({ command }) => command)).not.toContain(
    "get_meetings_storage",
  );
});

test("turning meetings off hides the setting", async ({ page }) => {
  await openAdvanced(page);
  await expect(page.getByText("Delete meeting audio")).toBeVisible();

  await appGroup(page)
    .getByRole("checkbox", { name: "Meetings" })
    .uncheck({ force: true });

  await expect(page.getByText("Delete meeting audio")).toHaveCount(0);
});

for (const theme of ["light", "dark"] as const) {
  test(`screenshot of the setting (${theme})`, async ({ page }) => {
    test.skip(!outputDir, "Set MURMUR_SCREENSHOT_DIR to capture screenshots");
    await page.emulateMedia({ colorScheme: theme });
    await page.setViewportSize(DEFAULT_WINDOW);
    await openAdvanced(page, `?theme=${theme}`);
    await expect(retentionRow(page)).toContainText("Meeting audio uses");
    // The sidebar's selection slides over to Advanced.
    await page.waitForTimeout(500);
    await page.screenshot({ path: `${outputDir}/advanced-${theme}.png` });

    await retentionRow(page).getByRole("button", { name: "Never" }).click();
    await expect(
      page.getByRole("option", { name: "After 90 days" }),
    ).toBeVisible();
    await page.screenshot({ path: `${outputDir}/advanced-open-${theme}.png` });
  });
}
