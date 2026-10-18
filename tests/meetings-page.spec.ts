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

test("meetings are on by default and the switch to turn them off is not experimental", async ({
  page,
}) => {
  await page.goto(fixture);
  await expect(sidebarButton(page, "Meetings")).toBeVisible();

  await sidebarButton(page, "Advanced").click();
  const appGroup = page.locator("section").filter({
    has: page.getByRole("heading", { name: "App", exact: true }),
  });
  const meetingsSwitch = appGroup.getByRole("checkbox", { name: "Meetings" });
  await expect(meetingsSwitch).toBeChecked();

  await meetingsSwitch.uncheck({ force: true });
  await expect(sidebarButton(page, "Meetings")).toHaveCount(0);
  expect(await calls(page)).toContain("change_meetings_enabled_setting");
});

test("the Meetings page is hidden while meetings are off", async ({ page }) => {
  await page.goto(`${fixture}?meetings=0`);
  await expect(sidebarButton(page, "History")).toBeVisible();
  await expect(sidebarButton(page, "Meetings")).toHaveCount(0);
});

test("lists meetings newest first with their duration and status", async ({
  page,
}) => {
  await openMeetings(page);

  const items = page.getByRole("listitem");
  await expect(items).toHaveCount(3);
  await expect(items.nth(0)).toContainText("Meeting · ");
  await expect(items.nth(0)).toContainText("47:12 · Processing");
  await expect(items.nth(1)).toContainText("Point hebdo produit");
  await expect(items.nth(1)).toContainText("1:12:05 · Ready");
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

test("a list load already on its way does not bring a deleted meeting back", async ({
  page,
}) => {
  await openMeetings(page);
  const items = page.getByRole("listitem");
  await expect(items).toHaveCount(3);

  const loads = async () =>
    (await calls(page)).filter((command) => command === "get_meetings").length;
  const before = await loads();
  // A progress update starts a load that answers after the delete.
  await page.evaluate(() => {
    window.settingsPages.holdMeetingLoads();
    void window.settingsPages.emitMeetingsUpdated(4);
  });
  await expect.poll(loads).toBeGreaterThan(before);

  await items.nth(1).getByRole("button", { name: "Delete Meeting" }).click();
  await page
    .getByRole("dialog")
    .getByRole("button", { name: "Delete" })
    .click();
  await expect(items).toHaveCount(2);

  await page.evaluate(() => window.settingsPages.releaseMeetingLoads());
  await page.waitForTimeout(100);
  await expect(items).toHaveCount(2);
  await expect(page.getByText("Point hebdo produit")).toHaveCount(0);
});

test("an update keeps every page of the list that is shown", async ({
  page,
}) => {
  await openMeetings(page, "&manyMeetings=1");
  const items = page.getByRole("listitem");
  await expect(items).toHaveCount(50);

  await page.getByRole("button", { name: "Show More" }).click();
  await expect(items).toHaveCount(63);

  const loads = async () =>
    (await calls(page)).filter((command) => command === "get_meetings").length;
  const before = await loads();
  // A meeting still processing reports progress.
  await page.evaluate(() => window.settingsPages.emitMeetingsUpdated(4));
  await expect.poll(loads).toBeGreaterThan(before);
  await page.waitForTimeout(100);
  await expect(items).toHaveCount(63);
  await expect(page.getByRole("button", { name: "Show More" })).toHaveCount(0);
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

test("discarding names the meeting, and the dialog closes once that meeting stops", async ({
  page,
}) => {
  await openMeetings(page, "&meeting=recording");

  await page.getByRole("button", { name: "Discard" }).click();
  await page
    .getByRole("dialog")
    .getByRole("button", { name: "Discard" })
    .click();
  await expect
    .poll(() =>
      page.evaluate(() =>
        window.settingsPages.calls.find(
          ({ command }) => command === "discard_meeting",
        ),
      ),
    )
    .toEqual({ command: "discard_meeting", args: { id: 4 } });

  await page.getByRole("button", { name: "Discard" }).click();
  await expect(page.getByRole("dialog")).toBeVisible();
  await page.evaluate(() =>
    window.settingsPages.emitMeetingState({ state: "idle" }),
  );
  await expect(page.getByRole("dialog")).toHaveCount(0);
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
