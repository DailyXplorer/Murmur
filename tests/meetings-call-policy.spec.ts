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

const rulesGroup = (page: Page) =>
  page.locator("section").filter({
    has: page.getByRole("heading", { name: "When a call starts" }),
  });

/** The innermost block holding both an app's name and its menu. */
const appRow = (page: Page, app: string) =>
  rulesGroup(page)
    .locator("div")
    .filter({ has: page.getByRole("heading", { name: app, exact: true }) })
    .filter({ has: page.locator('[aria-haspopup="listbox"]') })
    .last();

const calls = (page: Page) => page.evaluate(() => window.settingsPages.calls);

const APPS = [
  "Zoom",
  "Google Meet",
  "Microsoft Teams",
  "Slack",
  "FaceTime",
  "Webex",
  "Discord",
];

test("every call app asks by default", async ({ page }) => {
  await openAdvanced(page);

  for (const app of APPS) {
    await expect(
      appRow(page, app).getByRole("button", { name: "Ask", exact: true }),
    ).toBeVisible();
  }
  expect(
    await page.evaluate(() => window.settingsPages.unhandledCommands),
  ).toEqual([]);
});

test("the saved rules show on their apps", async ({ page }) => {
  await openAdvanced(page, "?callRules=zoom:always,face_time:never");

  await expect(
    appRow(page, "Zoom").getByRole("button", { name: "Always record" }),
  ).toBeVisible();
  await expect(
    appRow(page, "FaceTime").getByRole("button", { name: "Never ask" }),
  ).toBeVisible();
  await expect(
    appRow(page, "Slack").getByRole("button", { name: "Ask", exact: true }),
  ).toBeVisible();
});

test("choosing a rule saves it with the others", async ({ page }) => {
  await openAdvanced(page, "?callRules=face_time:never");

  await appRow(page, "Zoom")
    .getByRole("button", { name: "Ask", exact: true })
    .click();
  await page.getByRole("option", { name: "Always record" }).click();

  await expect(
    appRow(page, "Zoom").getByRole("button", { name: "Always record" }),
  ).toBeVisible();
  expect(await calls(page)).toContainEqual({
    command: "change_meeting_call_policies_setting",
    args: { policies: { face_time: "never", zoom: "always" } },
  });
});

test("the rules are hidden while call prompts or meetings are off", async ({
  page,
}) => {
  await openAdvanced(page, "?callPrompt=0");
  await expect(page.getByText("Offer to record calls")).toBeVisible();
  await expect(page.getByText("When a call starts")).toHaveCount(0);

  await openAdvanced(page, "?meetings=0");
  await expect(page.getByText("When a call starts")).toHaveCount(0);
});

test("the rules are hidden on a Mac that cannot tell a call started", async ({
  page,
}) => {
  await openAdvanced(page, "?callDetection=0");
  await expect(page.getByText("Delete meeting audio")).toBeVisible();

  await expect(page.getByText("When a call starts")).toHaveCount(0);
});

for (const theme of ["light", "dark"] as const) {
  test(`screenshot of the rules (${theme})`, async ({ page }) => {
    test.skip(!outputDir, "Set MURMUR_SCREENSHOT_DIR to capture screenshots");
    await page.emulateMedia({ colorScheme: theme });
    await page.setViewportSize({
      width: DEFAULT_WINDOW.width,
      height: DEFAULT_WINDOW.height + 360,
    });
    await openAdvanced(
      page,
      `?theme=${theme}&callRules=zoom:always,face_time:never,slack:never`,
    );
    await expect(rulesGroup(page)).toBeVisible();
    // The sidebar's selection slides over to Advanced.
    await page.waitForTimeout(500);
    await rulesGroup(page).scrollIntoViewIfNeeded();
    await page.screenshot({ path: `${outputDir}/call-rules-${theme}.png` });
  });
}
