import { expect, test } from "@playwright/test";
import {
  dispatch,
  entranceFinished,
  LIVE_TEXT_WINDOW,
  openOverlay,
} from "./overlayHarness";

const outputDir = process.env.MURMUR_SCREENSHOT_DIR;

// The overlay window is transparent. A mid-grey stands in for the desktop,
// so the dark pill and its shadow both show.
const DESKTOP = "rgb(128 128 128)";

const SHOTS = [
  ["short", "Thanks for joining"],
  [
    "clipped",
    "Thanks for joining, we will go through the plan for the next quarter and then take questions at the very end",
  ],
  [
    "arabic",
    "مرحبا بكم جميعا في هذا الاجتماع وسوف نتحدث اليوم عن خطة العمل للربع القادم ثم نجيب عن الأسئلة في النهاية",
  ],
] as const;

test.describe("live text screenshots", () => {
  test.skip(!outputDir, "Set MURMUR_SCREENSHOT_DIR to capture screenshots");
  test.use({ viewport: LIVE_TEXT_WINDOW, deviceScaleFactor: 2 });

  for (const theme of ["light", "dark"] as const) {
    test(theme, async ({ page }) => {
      await page.emulateMedia({ colorScheme: theme });
      await openOverlay(page);
      await page.evaluate(
        (background) => (document.body.style.background = background),
        DESKTOP,
      );
      await page.evaluate(() => document.fonts.ready);
      await dispatch(page, "show-overlay", "recording");
      await dispatch(page, "recording-ready");
      await expect(page.locator(".ov-capsule")).toHaveClass(/live/);
      await entranceFinished(page);

      for (const [name, text] of SHOTS) {
        await dispatch(page, "dictation-live-text", text);
        await expect(page.locator(".ov-live-words")).toHaveText(text);
        await page.screenshot({
          path: `${outputDir}/overlay-live-text-${name}-${theme}.png`,
        });
      }

      await dispatch(page, "dictation-live-text", SHOTS[1][1]);
      await dispatch(page, "show-overlay", "transcribing");
      await expect(page.locator(".ov-live-text")).toHaveCSS("opacity", "0.55");
      await page.screenshot({
        path: `${outputDir}/overlay-live-text-transcribing-${theme}.png`,
      });
    });
  }
});
