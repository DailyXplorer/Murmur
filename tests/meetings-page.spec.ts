import { expect, test, type Locator, type Page } from "@playwright/test";
import { readdirSync, readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

const fixture = "/tests/fixtures/settings-pages.html";

const localesDir = fileURLToPath(
  new URL("../src/i18n/locales/", import.meta.url),
);
const LOCALES = readdirSync(localesDir).map((locale) => {
  const { sidebar, meetings } = JSON.parse(
    readFileSync(`${localesDir}${locale}/translation.json`, "utf8"),
  ) as {
    sidebar: { meetings: string };
    meetings: { permission: { title: string } };
  };
  return { locale, page: sidebar.meetings, title: meetings.permission.title };
});

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

test("a Discard dialog its meeting outlived never opens for the next meeting", async ({
  page,
}) => {
  await openMeetings(page, "&meeting=recording");

  await page.getByRole("button", { name: "Discard" }).click();
  await expect(page.getByRole("dialog")).toBeVisible();
  // The meeting stops from the shortcut instead, the user deletes it, and
  // the next meeting gets its id back.
  await page.evaluate(() =>
    window.settingsPages.emitMeetingState({ state: "idle" }),
  );
  await expect(page.getByRole("dialog")).toHaveCount(0);
  await page.evaluate(() =>
    window.settingsPages.emitMeetingState({
      id: 4,
      started_at_ms: Date.now(),
      state: "recording",
      stops_at_ms: Date.now() + 3 * 3_600_000,
      tracks: "both",
    }),
  );
  await expect(
    page.getByRole("button", { name: "Stop", exact: true }),
  ).toBeVisible();
  await page.waitForTimeout(100);
  await expect(page.getByRole("dialog")).toHaveCount(0);
  expect(await calls(page)).not.toContain("discard_meeting");
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

test("the page starts with the recording controls, and its buttons show text only", async ({
  page,
}) => {
  await openMeetings(page);
  await expect(page.getByText(/Transcripts and summaries appear/)).toHaveCount(
    0,
  );
  const start = page.getByRole("button", { name: "Start Meeting" });
  await expect(start).toBeVisible();
  await expect(start.locator("svg")).toHaveCount(0);

  await openMeetings(page, "&meeting=recording");
  const stop = page.getByRole("button", { name: "Stop", exact: true });
  await expect(stop).toBeVisible();
  await expect(stop.locator("svg")).toHaveCount(0);
});

const speakerGroup = (page: Page) =>
  page.locator("section").filter({
    has: page.getByRole("heading", { name: "Speaker Detection", exact: true }),
  });

test("speaker detection has no licences button, in any state", async ({
  page,
}) => {
  for (const models of ["missing", "downloading", "ready"]) {
    await openMeetings(page, `&models=${models}`);
    await expect(speakerGroup(page).getByRole("button")).not.toHaveCount(0);
    await expect(page.getByRole("button", { name: /Licences/ })).toHaveCount(0);
  }
});

test("the download progress sits in the models row, under its title", async ({
  page,
}) => {
  await openMeetings(page, "&models=downloading");
  const group = speakerGroup(page);
  const title = group.getByRole("heading", { name: "Downloading… 42%" });
  const bar = group.getByRole("progressbar", { name: "Downloading… 42%" });
  const cancel = group.getByRole("button", { name: "Cancel" });
  await expect(bar).toHaveAttribute("aria-valuenow", "42");

  const [titleBox, barBox, cancelBox] = await Promise.all([
    title.boundingBox(),
    bar.boundingBox(),
    cancel.boundingBox(),
  ]);
  if (!titleBox || !barBox || !cancelBox) throw new Error("not laid out");
  // In the text column: the title's left edge, below it, clear of Cancel.
  expect(Math.abs(barBox.x - titleBox.x)).toBeLessThanOrEqual(1);
  expect(barBox.y).toBeGreaterThanOrEqual(titleBox.y + titleBox.height);
  expect(barBox.x + barBox.width).toBeLessThan(cancelBox.x);
});

/** A double click whose second click lands once the button has turned into
 * the next action, as it does in the app within a few milliseconds. */
const doubleClickAcrossStates = async (
  page: Page,
  first: Locator,
  second: Locator,
) => {
  const box = await first.boundingBox();
  if (!box) throw new Error("not laid out");
  await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
  await page.mouse.down({ clickCount: 1 });
  await page.mouse.up({ clickCount: 1 });
  await expect(second).toBeVisible();
  await page.mouse.down({ clickCount: 2 });
  await page.mouse.up({ clickCount: 2 });
};

test("a double click on a speaker models button runs only its first action", async ({
  page,
}) => {
  const actions = async () =>
    (await calls(page)).filter(
      (command) =>
        command.includes("meeting_models") &&
        command !== "get_meeting_models_status",
    );

  const button = (name: string) =>
    speakerGroup(page).getByRole("button", { name, exact: true });

  await openMeetings(page, "&models=missing");
  await doubleClickAcrossStates(page, button("Download"), button("Cancel"));
  expect(await actions()).toEqual(["download_meeting_models"]);

  await openMeetings(page, "&models=ready");
  await doubleClickAcrossStates(
    page,
    button("Delete Models"),
    button("Download"),
  );
  expect(await actions()).toEqual(["delete_meeting_models"]);
});

test("the system audio row's title fits at the default window size in every language", async ({
  page,
}) => {
  await page.setViewportSize({ width: 680, height: 570 });
  for (const { locale, page: meetings, title } of LOCALES) {
    await page.goto(`${fixture}?meetings=1&lang=${locale}`);
    await sidebarButton(page, meetings).click();
    const heading = page.getByRole("heading", { name: title, exact: true });
    // Room left beside the title and its info button. macOS draws text a
    // little wider than this browser, so a bare fit is not enough.
    const room = await heading.evaluate((element) => {
      const line = element.parentElement as HTMLElement;
      const used = Array.from(line.children).reduce(
        (sum, child) => sum + (child as HTMLElement).offsetWidth,
        element.scrollWidth - element.offsetWidth,
      );
      return line.clientWidth - used;
    });
    expect.soft(room, `${locale}: ${title}`).toBeGreaterThanOrEqual(16);
  }
});

test("the speaker models' licences open from About", async ({ page }) => {
  await page.goto(fixture);
  await sidebarButton(page, "About").click();
  await page.getByRole("button", { name: "View Licences" }).click();
  const dialog = page.getByRole("dialog", { name: "Model Licences" });
  await expect(dialog).toContainText("pyannote, CC BY 4.0");
  await expect(dialog).toContainText("Murmur doesn't include them.");
});
