import { expect, test, type Page } from "@playwright/test";
import { MEETING_LOCALES, untitledNotice } from "./meetingNoticeFixtures";
import { dispatch, openOverlay } from "./overlayHarness";

const fixture = "/tests/fixtures/settings-pages.html";

const reportedMeeting = (page: Page) =>
  page.evaluate(
    () =>
      window.settingsPages.calls
        .filter(({ command }) => command === "report_opened_meeting")
        .at(-1)?.args,
  );

const expectMeeting = async (page: Page, id: number) => {
  await expect(
    page.getByRole("button", { name: "All meetings", exact: true }),
  ).toBeVisible();
  await expect.poll(() => reportedMeeting(page)).toEqual({ id });
  await expect(
    page
      .getByRole("navigation")
      .getByRole("button", { name: "Meetings", exact: true }),
  ).toHaveAttribute("aria-current", "page");
};

for (const strict of [false, true]) {
  test(`a pending Open action opens its meeting after the Settings window is recreated${strict ? " in StrictMode" : ""}`, async ({
    page,
  }) => {
    await page.goto(
      `${fixture}?openMeeting=3&lastSection=history&strict=${strict ? "1" : "0"}`,
    );
    await expect(
      page.getByRole("textbox", { name: "Meeting title", exact: true }),
    ).toHaveValue("Point hebdo produit");
    await expectMeeting(page, 3);
    expect(
      await page.evaluate(() => window.settingsPages.unhandledCommands),
    ).toEqual([]);
  });
}

test("Open navigates an existing Settings page and a batch returns to the Meetings list", async ({
  page,
}) => {
  await page.goto(fixture);
  await expect(page.getByRole("navigation")).toBeVisible();
  await page.evaluate(() =>
    window.settingsPages.openMeetingNotice({ meeting: 3 }),
  );
  await expectMeeting(page, 3);
  await expect(
    page.getByRole("textbox", { name: "Meeting title", exact: true }),
  ).toHaveValue("Point hebdo produit");

  await page.evaluate(() =>
    window.settingsPages.openMeetingNotice({ meeting: 4 }),
  );
  await expectMeeting(page, 4);
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          window.settingsPages.calls
            .filter(({ command }) => command === "get_meeting")
            .at(-1)?.args,
      ),
    )
    .toEqual({ id: 4 });

  await page.evaluate(() => window.settingsPages.openMeetingNotice("meetings"));
  await expect(
    page.getByRole("heading", { name: "Recordings", exact: true }),
  ).toBeVisible();
  await expect(
    page.getByRole("textbox", { name: "Meeting title", exact: true }),
  ).toHaveCount(0);
  await expect.poll(() => reportedMeeting(page)).toEqual({ id: null });
  expect(
    await page.evaluate(() => window.settingsPages.unhandledCommands),
  ).toEqual([]);
});

test("a pending batch Open action opens the Meetings list", async ({
  page,
}) => {
  await page.goto(`${fixture}?openMeeting=all&lastSection=history`);
  await expect(
    page.getByRole("heading", { name: "Recordings", exact: true }),
  ).toBeVisible();
  await expect.poll(() => reportedMeeting(page)).toEqual({ id: null });
  expect(
    await page.evaluate(() =>
      window.settingsPages.calls.some(
        ({ command }) => command === "get_meeting",
      ),
    ),
  ).toBe(false);
});

test("the page reports a manually opened meeting and clears it on Back and when leaving Meetings", async ({
  page,
}) => {
  await page.goto(`${fixture}?lastSection=meetings`);
  const meeting = page.getByRole("button", { name: /Point hebdo produit/ });
  await meeting.click();
  await expectMeeting(page, 3);
  await page.getByRole("button", { name: "All meetings", exact: true }).click();
  await expect.poll(() => reportedMeeting(page)).toEqual({ id: null });
  await meeting.click();
  await expectMeeting(page, 3);
  await page
    .getByRole("navigation")
    .getByRole("button", { name: "General", exact: true })
    .click();
  await expect.poll(() => reportedMeeting(page)).toEqual({ id: null });
});

for (const locale of ["en", "fr", "de", "es", "it", "pt"] as const) {
  test(`the ready and failed default names match the Meetings list in ${locale}`, async ({
    page,
    context,
  }) => {
    const strings = MEETING_LOCALES[locale];
    await page.goto(`${fixture}?lastSection=meetings&lang=${locale}`);
    await expect(
      page.getByRole("heading", {
        name: strings.meetings.list.title,
        exact: true,
      }),
    ).toBeVisible();
    const listName = await page
      .getByRole("button", { name: / · / })
      .first()
      .locator("p")
      .first()
      .innerText();
    const overlay = await context.newPage();
    await openOverlay(overlay);
    const startedAtMs = 1_760_000_000_000 - 50 * 60_000;
    for (const failed of [false, true]) {
      await dispatch(
        overlay,
        "show-overlay-meeting-notice",
        untitledNotice(locale, startedAtMs, failed),
      );
      await expect(overlay.getByText(listName, { exact: true })).toBeVisible();
      await expect(
        overlay.locator(
          failed ? ".ov-meeting-notice-detail" : ".ov-meeting-notice-headline",
        ),
      ).toHaveText(listName);
    }
    await overlay.close();
  });
}
