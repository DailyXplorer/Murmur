import { expect, test, type Page } from "@playwright/test";
import { MEETING_NOTICES } from "./meetingNoticeFixtures";
import {
  dispatch,
  entranceFinished,
  fitCopyWindow,
  openOverlay,
  widthsAfter,
} from "./overlayHarness";

const calls = (page: Page) =>
  page.evaluate(() => window.overlayHarness.meetingNoticeCalls());

test.describe("meeting notice", () => {
  test("replays a meeting notice that was available before the overlay mounted", async ({
    page,
  }) => {
    await openOverlay(page, { meetingNotice: MEETING_NOTICES.ready });
    const notice = page.getByRole("status");
    await expect(
      notice.getByText(MEETING_NOTICES.ready.headline, { exact: true }),
    ).toBeVisible();
    await expect(
      notice.getByText("Transcript ready", { exact: true }),
    ).toBeVisible();
    await expect(
      notice.getByRole("button", { name: "Open", exact: true }),
    ).toBeVisible();
    await expect(
      notice.getByRole("button", { name: "Copy", exact: true }),
    ).toBeVisible();
    expect(await calls(page)).toEqual([]);
  });

  test("shows the ready meeting with named actions and fits its window", async ({
    page,
  }) => {
    await openOverlay(page);
    await dispatch(page, "show-overlay-meeting-notice", MEETING_NOTICES.ready);

    const notice = page.getByRole("status");
    await expect(notice).toBeVisible();
    await expect(
      notice.getByText(MEETING_NOTICES.ready.headline, { exact: true }),
    ).toBeVisible();
    await expect(
      notice.getByText("Transcript ready", { exact: true }),
    ).toBeVisible();
    await expect(
      notice.getByRole("button", { name: "Open", exact: true }),
    ).toBeVisible();
    await expect(
      notice.getByRole("button", { name: "Copy", exact: true }),
    ).toBeVisible();
    await expect(
      notice.getByRole("button", { name: "Close", exact: true }),
    ).toHaveAttribute("aria-label", "Close");
    await expect(notice.getByRole("button")).toHaveCount(3);
    await expect(notice.locator(".ov-glyph.done")).toHaveCount(1);
    expect(await calls(page)).toEqual([]);

    await entranceFinished(page);
    const fitted = await fitCopyWindow(page);
    const bounds = await notice.evaluate((capsule) => {
      const { width, height } = capsule.getBoundingClientRect();
      return { width, height };
    });
    expect(fitted).toEqual({
      width: Math.ceil(bounds.width) + 32,
      height: Math.ceil(bounds.height) + 32,
    });
    expect(bounds.height).toBe(48);
  });

  test("Copy confirms with a check without moving the pill or the other actions", async ({
    page,
  }) => {
    await openOverlay(page);
    await dispatch(page, "show-overlay-meeting-notice", MEETING_NOTICES.ready);
    await entranceFinished(page);
    const before = await fitCopyWindow(page);
    const open = page.getByRole("button", { name: "Open", exact: true });
    const beforeOpen = await open.boundingBox();
    const copy = page.getByRole("button", { name: "Copy", exact: true });
    await expect(copy.locator("svg")).toHaveAttribute("data-icon", "copy");

    await copy.click();
    const copied = page.getByRole("button", { name: "Copied", exact: true });
    await expect(copied).toBeVisible();
    await expect(copied.locator("svg")).toHaveAttribute("data-icon", "check");
    expect(await calls(page)).toEqual(["copy_meeting_notice"]);
    expect(await fitCopyWindow(page)).toEqual(before);
    expect(await open.boundingBox()).toEqual(beforeOpen);
    await expect(copy).toBeVisible({ timeout: 3000 });
    await expect(page.getByRole("status")).toBeVisible();
    expect(await open.boundingBox()).toEqual(beforeOpen);
  });

  test("a refused copy leaves Copy visible", async ({ page }) => {
    const warnings: string[] = [];
    page.on("console", (message) => {
      if (message.type() === "warning") warnings.push(message.text());
    });
    await openOverlay(page, {
      copyError: "copy_meeting_notice not allowed",
    });
    await dispatch(page, "show-overlay-meeting-notice", MEETING_NOTICES.ready);
    await page.getByRole("button", { name: "Copy", exact: true }).click();

    await expect
      .poll(() => warnings.join("\n"))
      .toContain("copy_meeting_notice not allowed");
    await expect(
      page.getByRole("button", { name: "Copy", exact: true }),
    ).toBeVisible();
    await expect(
      page.getByRole("button", { name: "Copied", exact: true }),
    ).toHaveCount(0);
    expect(await calls(page)).toEqual(["copy_meeting_notice"]);
  });

  test("Open and close invoke their native commands and wait for native dismissal", async ({
    page,
  }) => {
    await openOverlay(page);
    await dispatch(page, "show-overlay-meeting-notice", MEETING_NOTICES.ready);
    await page.getByRole("button", { name: "Open", exact: true }).click();
    await page.getByRole("button", { name: "Close", exact: true }).click();

    expect(await calls(page)).toEqual([
      "open_meeting_notice",
      "dismiss_meeting_notice",
    ]);
    await expect(page.getByRole("status")).toBeVisible();
    await dispatch(page, "hide-overlay");
    await expect(page.locator(".ov-stage")).toHaveCount(0);
  });

  test("the failed meeting uses the warning glyph with Open and no Copy", async ({
    page,
  }) => {
    await openOverlay(page);
    await dispatch(page, "show-overlay-meeting-notice", MEETING_NOTICES.failed);

    const notice = page.getByRole("status");
    await expect(notice).toHaveClass(/ov-warning/);
    await expect(
      notice.getByText(MEETING_NOTICES.failed.headline),
    ).toBeVisible();
    await expect(
      notice.getByText(MEETING_NOTICES.failed.detail, { exact: true }),
    ).toBeVisible();
    await expect(notice.locator(".ov-glyph.off")).toHaveCount(1);
    await expect(notice.getByRole("button")).toHaveCount(2);
    await expect(
      notice.getByRole("button", { name: "Copy", exact: true }),
    ).toHaveCount(0);
    await notice.getByRole("button", { name: "Open", exact: true }).click();
    expect(await calls(page)).toEqual(["open_meeting_notice"]);
  });

  for (const state of ["batch", "mixed", "failedBatch"] as const) {
    test(`the ${state} pill shows its supplied detail and no Copy`, async ({
      page,
    }) => {
      await openOverlay(page);
      const labels = MEETING_NOTICES[state];
      await dispatch(page, "show-overlay-meeting-notice", labels);

      const notice = page.getByRole("status");
      await expect(
        notice.getByText(labels.headline, { exact: true }),
      ).toBeVisible();
      if (labels.detail === null) {
        await expect(notice.locator(".ov-meeting-notice-detail")).toHaveCount(
          0,
        );
      } else {
        await expect(
          notice.getByText(labels.detail, { exact: true }),
        ).toBeVisible();
      }
      await expect(notice.getByRole("button")).toHaveCount(2);
      await expect(
        notice.getByRole("button", { name: "Copy", exact: true }),
      ).toHaveCount(0);
      await notice.getByRole("button", { name: "Open", exact: true }).click();
      expect(await calls(page)).toEqual(["open_meeting_notice"]);
    });
  }

  test("opens from the orb once, then updates the title and batch in place and fits again", async ({
    page,
  }) => {
    await openOverlay(page);
    await page.evaluate(() => document.fonts.ready);
    const widths = await widthsAfter(
      page,
      "show-overlay-meeting-notice",
      MEETING_NOTICES.ready,
      ".ov-meeting-notice",
    );
    expect(widths[0]).toBeLessThanOrEqual(40);
    expect(widths[widths.length - 1]).toBeGreaterThan(250);

    const fitsBefore = await page.evaluate(
      () => window.overlayHarness.copyFits().length,
    );
    const updated = { ...MEETING_NOTICES.ready, headline: "Planning" };
    const updateWidths = await widthsAfter(
      page,
      "show-overlay-meeting-notice",
      updated,
      ".ov-meeting-notice",
      updated.headline,
    );
    expect(Math.min(...updateWidths)).toBeGreaterThan(200);
    expect(new Set(updateWidths).size).toBe(1);
    await expect(
      page.getByText(updated.headline, { exact: true }),
    ).toBeVisible();
    await expect
      .poll(() => page.evaluate(() => window.overlayHarness.copyFits().length))
      .toBeGreaterThan(fitsBefore);

    const batchWidths = await widthsAfter(
      page,
      "show-overlay-meeting-notice",
      MEETING_NOTICES.batch,
      ".ov-meeting-notice",
      MEETING_NOTICES.batch.headline,
    );
    expect(Math.min(...batchWidths)).toBeGreaterThan(150);
    expect(new Set(batchWidths).size).toBe(1);
    await expect(
      page.getByRole("button", { name: "Copy", exact: true }),
    ).toHaveCount(0);
  });

  test("a long title ellipsizes while its detail and actions stay inside the fitted pill", async ({
    page,
  }) => {
    await openOverlay(page);
    await page.evaluate(() => document.fonts.ready);
    await dispatch(page, "show-overlay-meeting-notice", MEETING_NOTICES.long);
    await entranceFinished(page);
    const fitted = await fitCopyWindow(page);
    const headline = page.getByText(MEETING_NOTICES.long.headline, {
      exact: true,
    });
    await expect(headline).toHaveCSS("text-overflow", "ellipsis");
    await expect(headline).toHaveCSS("white-space", "nowrap");
    const titleSize = await headline.evaluate((element) => ({
      client: element.clientWidth,
      scroll: element.scrollWidth,
    }));
    expect(titleSize.scroll).toBeGreaterThan(titleSize.client);
    expect(titleSize.client).toBeLessThanOrEqual(240);
    expect(fitted.width).toBeLessThanOrEqual(592);

    const inside = await page.getByRole("status").evaluate((capsule) => {
      const box = capsule.getBoundingClientRect();
      return {
        window:
          box.left >= 16 &&
          box.top >= 16 &&
          box.right <= innerWidth - 16 &&
          box.bottom <= innerHeight - 16,
        content: [
          ...capsule.querySelectorAll("button, .ov-meeting-notice-detail"),
        ].every((element) => {
          const rect = element.getBoundingClientRect();
          return (
            rect.left >= box.left &&
            rect.right <= box.right &&
            rect.top >= box.top &&
            rect.bottom <= box.bottom &&
            element.scrollWidth <= element.clientWidth
          );
        }),
      };
    });
    expect(inside).toEqual({ window: true, content: true });
  });

  test("a late copy reply cannot confirm a replacement notice", async ({
    page,
  }) => {
    await openOverlay(page, { holdMeetingCopy: true });
    await dispatch(page, "show-overlay-meeting-notice", MEETING_NOTICES.ready);
    await page.getByRole("button", { name: "Copy", exact: true }).click();
    expect(await calls(page)).toEqual(["copy_meeting_notice"]);
    const replacement = {
      ...MEETING_NOTICES.ready,
      meeting_id: 8,
      headline: "Next meeting",
    };
    await dispatch(page, "show-overlay-meeting-notice", replacement);
    await expect(
      page.getByText(replacement.headline, { exact: true }),
    ).toBeVisible();
    await page.evaluate(() => window.overlayHarness.releaseMeetingCopy());
    await expect(
      page.getByRole("button", { name: "Copy", exact: true }),
    ).toBeVisible();
    await expect(
      page.getByRole("button", { name: "Copied", exact: true }),
    ).toHaveCount(0);
  });

  test("a title refresh preserves Copied until the original 1.5 second deadline", async ({
    page,
  }) => {
    const now = Date.parse("2026-11-03T10:00:00Z");
    await page.clock.install({ time: now - 1000 });
    await page.clock.pauseAt(now);
    await page.emulateMedia({ reducedMotion: "reduce" });
    await openOverlay(page);
    const original = { ...MEETING_NOTICES.ready, meeting_id: 7 };
    await dispatch(page, "show-overlay-meeting-notice", original);
    await page.getByRole("button", { name: "Copy", exact: true }).click();
    const copied = page.getByRole("button", { name: "Copied", exact: true });
    await expect(copied).toBeVisible();
    await page.clock.runFor(500);
    await dispatch(page, "show-overlay-meeting-notice", {
      ...original,
      headline: "Planning",
    });
    await expect(copied).toBeVisible();
    await page.clock.runFor(999);
    await expect(copied).toBeVisible();
    await page.clock.runFor(1);
    await expect(
      page.getByRole("button", { name: "Copy", exact: true }),
    ).toBeVisible();
    await expect(page.getByText("Planning", { exact: true })).toBeVisible();
  });

  test("a copy reply still confirms the same meeting after its title arrives", async ({
    page,
  }) => {
    await openOverlay(page, { holdMeetingCopy: true });
    const original = { ...MEETING_NOTICES.ready, meeting_id: 7 };
    await dispatch(page, "show-overlay-meeting-notice", original);
    await page.getByRole("button", { name: "Copy", exact: true }).click();
    await dispatch(page, "show-overlay-meeting-notice", {
      ...original,
      headline: "Planning",
    });
    await page.evaluate(() => window.overlayHarness.releaseMeetingCopy());
    await expect(
      page.getByRole("button", { name: "Copied", exact: true }),
    ).toBeVisible();
    await expect(page.getByText("Planning", { exact: true })).toBeVisible();
  });

  test("native hide and every other overlay state replace the notice", async ({
    page,
  }) => {
    await openOverlay(page);
    const replacements = [
      {
        event: "hide-overlay",
        payload: null,
        selector: ".ov-stage",
        count: 0,
      },
      {
        event: "show-overlay",
        payload: "recording",
        selector: ".ov-dictation.arming",
        count: 1,
      },
      {
        event: "show-overlay",
        payload: "transcribing",
        selector: ".ov-dictation.processing",
        count: 1,
      },
      {
        event: "show-overlay-error",
        payload: "Transcription failed",
        selector: ".ov-error",
        count: 1,
      },
      {
        event: "show-overlay-copy",
        payload: { note: "No field", copy: "Copy", copied: "Copied" },
        selector: ".ov-copy",
        count: 1,
      },
      {
        event: "show-overlay-warning",
        payload: "Microphone disconnected",
        selector: ".ov-warning",
        count: 1,
      },
      {
        event: "show-overlay-cancelled",
        payload: { note: "Dictation cancelled", restore: "Restore" },
        selector: ".ov-copy",
        count: 1,
      },
      {
        event: "show-overlay-meeting",
        payload: {
          started_at_ms: Date.now(),
          stops_at_ms: Date.now() + 10_800_000,
        },
        selector: ".ov-meeting",
        count: 1,
      },
    ];
    for (const replacement of replacements) {
      await dispatch(
        page,
        "show-overlay-meeting-notice",
        MEETING_NOTICES.ready,
      );
      await expect(page.locator(".ov-meeting-notice")).toHaveCount(1);
      await dispatch(page, replacement.event, replacement.payload);
      await expect(page.locator(".ov-meeting-notice")).toHaveCount(0);
      await expect(page.locator(replacement.selector)).toHaveCount(
        replacement.count,
      );
    }
    expect(await calls(page)).toEqual([]);
  });

  test("reduced motion has no entrance animation or action movement", async ({
    page,
  }) => {
    await page.emulateMedia({ reducedMotion: "reduce" });
    await openOverlay(page);
    await dispatch(page, "show-overlay-meeting-notice", MEETING_NOTICES.ready);
    await expect(page.getByRole("status")).toBeVisible();
    expect(await page.evaluate(() => document.getAnimations().length)).toBe(0);
    await page.getByRole("button", { name: "Copy", exact: true }).click();
    await expect(
      page.getByRole("button", { name: "Copied", exact: true }),
    ).toBeVisible();
    expect(await page.evaluate(() => document.getAnimations().length)).toBe(0);
  });

  test("uses the supplied French labels for every action", async ({ page }) => {
    await openOverlay(page);
    await dispatch(page, "show-overlay-meeting-notice", MEETING_NOTICES.french);
    await expect(
      page.getByText("Transcription prête", { exact: true }),
    ).toBeVisible();
    await expect(
      page.getByRole("button", { name: "Ouvrir", exact: true }),
    ).toBeVisible();
    await expect(
      page.getByRole("button", { name: "Fermer", exact: true }),
    ).toHaveAttribute("aria-label", "Fermer");
    await page.getByRole("button", { name: "Copier", exact: true }).click();
    await expect(
      page.getByRole("button", { name: "Copié", exact: true }),
    ).toBeVisible();
  });
});
