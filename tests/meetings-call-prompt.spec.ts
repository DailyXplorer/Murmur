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

const callPrompt = (page: Page) =>
  appGroup(page).getByRole("checkbox", { name: "Offer to record calls" });

const calls = (page: Page) => page.evaluate(() => window.settingsPages.calls);

test("call prompts are on by default, below the Meetings switch", async ({
  page,
}) => {
  await openAdvanced(page);

  await expect(callPrompt(page)).toBeChecked();
  const meetings = await appGroup(page)
    .getByRole("checkbox", { name: "Meetings" })
    .elementHandle();
  const switches = appGroup(page).getByRole("checkbox");
  const order = await switches.evaluateAll(
    (boxes, meetings) => boxes.indexOf(meetings as HTMLInputElement),
    meetings,
  );
  await expect(switches.nth(order + 1)).toHaveAccessibleName(
    "Offer to record calls",
  );
  expect(
    await page.evaluate(() => window.settingsPages.unhandledCommands),
  ).toEqual([]);
});

test("turning call prompts off saves it", async ({ page }) => {
  await openAdvanced(page);

  await callPrompt(page).uncheck({ force: true });

  await expect(callPrompt(page)).not.toBeChecked();
  expect(await calls(page)).toContainEqual({
    command: "change_meeting_call_prompt_setting",
    args: { enabled: false },
  });
});

test("the setting is hidden while meetings are off", async ({ page }) => {
  await openAdvanced(page, "?meetings=0");
  await expect(
    appGroup(page).getByRole("checkbox", { name: "Meetings" }),
  ).not.toBeChecked();

  await expect(page.getByText("Offer to record calls")).toHaveCount(0);
});

test("the setting is hidden on a Mac that cannot tell a call started", async ({
  page,
}) => {
  await openAdvanced(page, "?callDetection=0");
  await expect(
    appGroup(page).getByRole("checkbox", { name: "Meetings" }),
  ).toBeChecked();

  await expect(page.getByText("Offer to record calls")).toHaveCount(0);
});

for (const theme of ["light", "dark"] as const) {
  test(`screenshot of the setting (${theme})`, async ({ page }) => {
    test.skip(!outputDir, "Set MURMUR_SCREENSHOT_DIR to capture screenshots");
    await page.emulateMedia({ colorScheme: theme });
    await page.setViewportSize(DEFAULT_WINDOW);
    await openAdvanced(page, `?theme=${theme}`);
    await expect(callPrompt(page)).toBeChecked();
    // The sidebar's selection slides over to Advanced.
    await page.waitForTimeout(500);
    await page.screenshot({
      path: `${outputDir}/advanced-call-prompt-${theme}.png`,
    });
  });
}
