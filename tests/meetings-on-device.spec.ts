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

const onDevice = (page: Page) =>
  page.getByRole("checkbox", { name: "On-device meeting transcript" });

const bubble = async (page: Page) => {
  await page
    .getByRole("button", { name: /^The local model writes the meeting/ })
    .click();
  return page.getByRole("tooltip");
};

test("ChatGPT transcribes meetings until the switch is turned on", async ({
  page,
}) => {
  await openAdvanced(page);
  await expect(onDevice(page)).not.toBeChecked();

  await onDevice(page).check({ force: true });

  await expect(onDevice(page)).toBeChecked();
  expect(await page.evaluate(() => window.settingsPages.calls)).toContainEqual({
    command: "change_meeting_on_device_setting",
    args: { enabled: true },
  });
  expect(
    await page.evaluate(() => window.settingsPages.unhandledCommands),
  ).toEqual([]);
});

test("the bubble says what stays on the Mac and what still goes to ChatGPT", async ({
  page,
}) => {
  await openAdvanced(page);

  const text = await bubble(page);

  await expect(text).toContainText("its audio stays on this Mac");
  await expect(text).toContainText("Only the text goes to ChatGPT");
  await expect(text).toContainText("not one of the 25 it covers");
});

test("the Windows text says this PC", async ({ page }) => {
  await openAdvanced(page, "?platform=windows");

  await expect(await bubble(page)).toContainText("its audio stays on this PC");
});

test("the setting is hidden while meetings are off", async ({ page }) => {
  await openAdvanced(page, "?meetings=0");

  await expect(page.getByText("On-device meeting transcript")).toHaveCount(0);
});

for (const theme of ["light", "dark"] as const) {
  test(`screenshot of the setting (${theme})`, async ({ page }) => {
    test.skip(!outputDir, "Set MURMUR_SCREENSHOT_DIR to capture screenshots");
    await page.emulateMedia({ colorScheme: theme });
    await page.setViewportSize(DEFAULT_WINDOW);
    await openAdvanced(page, `?theme=${theme}`);
    await onDevice(page).check({ force: true });
    await bubble(page);
    // The sidebar's selection slides over to Advanced.
    await page.waitForTimeout(500);
    await page.screenshot({
      path: `${outputDir}/advanced-meetings-on-device-${theme}.png`,
    });
  });
}
