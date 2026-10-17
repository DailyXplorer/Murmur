import { expect, test, type Page } from "@playwright/test";

const fixture = "/tests/fixtures/settings-pages.html";

const sidebarButton = (page: Page, name: string) =>
  page.getByRole("navigation").getByRole("button", { name, exact: true });

const openMeetings = async (page: Page, query = "") => {
  await page.goto(`${fixture}?meetings=1${query}`);
  await sidebarButton(page, "Meetings").click();
};

const calls = (page: Page) =>
  page.evaluate(() => window.settingsPages.calls.map(({ command }) => command));

test("the Meetings page is hidden until meetings are turned on", async ({
  page,
}) => {
  await page.goto(fixture);
  await expect(sidebarButton(page, "History")).toBeVisible();
  await expect(sidebarButton(page, "Meetings")).toHaveCount(0);

  await sidebarButton(page, "Advanced").click();
  await expect(page.getByText("Meetings", { exact: true })).toBeVisible();
});

test("lists meetings newest first with their duration and status", async ({
  page,
}) => {
  await openMeetings(page);

  const items = page.getByRole("listitem");
  await expect(items).toHaveCount(3);
  await expect(items.nth(0)).toContainText("Meeting · ");
  await expect(items.nth(0)).toContainText("47:12 · Recorded");
  await expect(items.nth(1)).toContainText("1:12:05 · Recorded");
  await expect(items.nth(2)).toContainText("09:41 · Interrupted");
  await expect(
    page.getByRole("button", { name: "Start Meeting" }),
  ).toBeVisible();
  expect(
    await page.evaluate(() => window.settingsPages.unhandledCommands),
  ).toEqual([]);
});

test("deleting asks first, then removes the meeting", async ({ page }) => {
  await openMeetings(page);
  const items = page.getByRole("listitem");
  await expect(items).toHaveCount(3);

  await items.nth(1).getByRole("button", { name: "Delete Meeting" }).click();
  const dialog = page.getByRole("dialog");
  await expect(dialog).toContainText("Delete this meeting?");
  await dialog.getByRole("button", { name: "Cancel" }).click();
  await expect(items).toHaveCount(3);
  expect(await calls(page)).not.toContain("delete_meeting");

  await items.nth(1).getByRole("button", { name: "Delete Meeting" }).click();
  await page
    .getByRole("dialog")
    .getByRole("button", { name: "Delete" })
    .click();
  await expect(items).toHaveCount(2);
  await expect(items.nth(1)).toContainText("Interrupted");
  const deleted = await page.evaluate(() =>
    window.settingsPages.calls.find(
      ({ command }) => command === "delete_meeting",
    ),
  );
  expect(deleted?.args).toEqual({ id: 3 });
});

test("the start button starts a meeting", async ({ page }) => {
  await openMeetings(page);
  await page.getByRole("button", { name: "Start Meeting" }).click();
  await expect.poll(() => calls(page)).toContain("start_meeting");
});

test("a meeting in progress shows its time, can be stopped and cannot be deleted", async ({
  page,
}) => {
  await openMeetings(page, "&meeting=recording");

  await expect(page.getByText(/^Recording · 12:3\d$/)).toBeVisible();
  const live = page.getByRole("listitem").nth(0);
  await expect(live).toContainText("· Recording");
  await expect(
    live.getByRole("button", { name: "Delete Meeting" }),
  ).toBeDisabled();

  await page.getByRole("button", { name: "Discard" }).click();
  await expect(page.getByRole("dialog")).toContainText("Discard this meeting?");
  await page.getByRole("button", { name: "Keep Recording" }).click();
  expect(await calls(page)).not.toContain("discard_meeting");

  await page.getByRole("button", { name: "Stop", exact: true }).click();
  await expect.poll(() => calls(page)).toContain("stop_meeting");
});

test("without system audio support the sources are fixed to the microphone", async ({
  page,
}) => {
  await openMeetings(page, "&systemAudio=os_too_old");

  await expect(
    page.getByText(/System audio needs macOS 14\.4 or later/),
  ).toBeVisible();
  const sources = page.getByRole("button", { name: "Microphone Only" });
  await expect(sources).toBeDisabled();
  await expect(page.getByRole("button", { name: "Open Settings" })).toHaveCount(
    0,
  );
});
