import { expect, test } from "@playwright/test";
import { MEETING_NOTICES } from "./meetingNoticeFixtures";
import {
  dispatch,
  entranceFinished,
  fitCopyWindow,
  openOverlay,
} from "./overlayHarness";

const outputDir = process.env.MURMUR_SCREENSHOT_DIR;
const DESKTOP = { light: "rgb(236 236 240)", dark: "rgb(30 30 34)" };
const screenshots = [
  ["ready", MEETING_NOTICES.ready],
  ["copied", MEETING_NOTICES.ready],
  ["failed", MEETING_NOTICES.failed],
  ["batch", MEETING_NOTICES.batch],
  ["mixed-batch", MEETING_NOTICES.mixed],
  ["long-title", MEETING_NOTICES.long],
  ["ready-fr", MEETING_NOTICES.french],
] as const;

test.describe("meeting notice screenshots", () => {
  test.skip(!outputDir, "Set MURMUR_SCREENSHOT_DIR to capture screenshots");
  test.use({ deviceScaleFactor: 2 });

  for (const theme of ["light", "dark"] as const) {
    test(theme, async ({ page }) => {
      await page.emulateMedia({ colorScheme: theme });
      await openOverlay(page);
      await page.evaluate(
        (background) => (document.body.style.background = background),
        DESKTOP[theme],
      );
      await page.evaluate(() => document.fonts.ready);

      for (const [state, labels] of screenshots) {
        await page.evaluate(
          (language) => (document.documentElement.lang = language),
          state === "ready-fr" ? "fr" : "en",
        );
        await dispatch(page, "show-overlay-meeting-notice", labels);
        await expect(
          page.getByRole("status").getByText(labels.headline, { exact: true }),
        ).toBeVisible();
        await entranceFinished(page);
        if (state === "copied") {
          await page
            .getByRole("button", { name: labels.copy, exact: true })
            .click();
          await expect(
            page.getByRole("button", { name: labels.copied, exact: true }),
          ).toBeVisible();
        }
        await fitCopyWindow(page);
        await page.mouse.move(0, 0);
        const name =
          state === "ready-fr" ? `ready-${theme}-fr` : `${state}-${theme}`;
        await page.screenshot({
          path: `${outputDir}/${name}.png`,
        });
      }
    });
  }
});
