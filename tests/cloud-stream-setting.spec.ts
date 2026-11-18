import { expect, test, type Page } from "@playwright/test";

const openDebug = async (page: Page) => {
  await page.goto("/tests/fixtures/settings-pages.html");
  await page
    .getByRole("navigation")
    .getByRole("button", { name: "Debug", exact: true })
    .click();
};

const stream = (page: Page) =>
  page.getByRole("checkbox", { name: "Stream dictations to ChatGPT" });

test("dictations stream to ChatGPT until the switch is turned off", async ({
  page,
}) => {
  await openDebug(page);
  await expect(stream(page)).toBeChecked();

  await stream(page).uncheck({ force: true });

  await expect(stream(page)).not.toBeChecked();
  expect(await page.evaluate(() => window.settingsPages.calls)).toContainEqual({
    command: "change_cloud_stream_enabled_setting",
    args: { enabled: false },
  });
  expect(
    await page.evaluate(() => window.settingsPages.unhandledCommands),
  ).toEqual([]);
});

test("the bubble says when the audio is sent, on and off", async ({ page }) => {
  await openDebug(page);

  await page
    .getByRole("button", { name: /^With ChatGPT transcription, sends/ })
    .click();

  const text = page.getByRole("tooltip");
  await expect(text).toContainText("sends the audio while you speak");
  await expect(text).toContainText(
    "Off: the recording is uploaded after you stop.",
  );
});
