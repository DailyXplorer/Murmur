import { expect, test } from "@playwright/test";
import { readFileSync } from "node:fs";
import {
  dispatch,
  entranceFinished,
  ERROR_WINDOW,
  fitCopyWindow,
  openOverlay,
} from "./overlayHarness";

const outputDir = process.env.MURMUR_SCREENSHOT_DIR;

// The overlay window is transparent; these stand in for the desktop behind it.
const DESKTOP = { light: "rgb(236 236 240)", dark: "rgb(30 30 34)" };

const trayStrings = (locale: string) =>
  (
    JSON.parse(
      readFileSync(
        new URL(
          `../src/i18n/locales/${locale}/translation.json`,
          import.meta.url,
        ),
        "utf8",
      ),
    ) as { tray: Record<string, string> }
  ).tray;

const trayLabels = (locale: string) => {
  const tray = trayStrings(locale);
  return {
    copy: tray.overlayCopy,
    copied: tray.overlayCopied,
    note: tray.overlayNotPasted,
  };
};

const LOCAL_ERRORS = [
  ["model-not-ready", "errorLocalNotReady"],
  ["local-failed", "errorLocalFailed"],
  ["needs-cloud", "errorLanguageNeedsCloud"],
] as const;

test.describe("local engine error screenshots", () => {
  test.skip(!outputDir, "Set MURMUR_SCREENSHOT_DIR to capture screenshots");
  test.use({ viewport: ERROR_WINDOW, deviceScaleFactor: 2 });

  for (const locale of ["en", "fr"]) {
    for (const theme of ["light", "dark"] as const) {
      test(`${locale} ${theme}`, async ({ page }) => {
        await page.emulateMedia({ colorScheme: theme });
        await openOverlay(page);
        await page.evaluate(
          (background) => (document.body.style.background = background),
          DESKTOP[theme],
        );
        await page.evaluate(() => document.fonts.ready);
        const tray = trayStrings(locale);
        for (const [name, key] of LOCAL_ERRORS) {
          await dispatch(page, "show-overlay", "transcribing");
          await dispatch(page, "show-overlay-error", tray[key]);
          await expect(page.getByRole("alert")).toHaveText(tray[key]);
          await entranceFinished(page);
          await page.screenshot({
            path: `${outputDir}/overlay-error-${name}-${locale}-${theme}.png`,
          });
        }
      });
    }
  }
});

test.describe("copy pill screenshots", () => {
  test.skip(!outputDir, "Set MURMUR_SCREENSHOT_DIR to capture screenshots");
  test.use({ deviceScaleFactor: 2 });

  for (const locale of ["en", "fr", "de"]) {
    for (const theme of ["light", "dark"] as const) {
      test(`${locale} ${theme}`, async ({ page }) => {
        await page.emulateMedia({ colorScheme: theme });
        await openOverlay(page);
        await page.evaluate(
          (background) => (document.body.style.background = background),
          DESKTOP[theme],
        );
        await page.evaluate(() => document.fonts.ready);
        const labels = trayLabels(locale);

        await dispatch(page, "show-overlay-copy", labels);
        const button = page.getByRole("button", {
          name: labels.copy,
          exact: true,
        });
        await expect(button).toBeVisible();
        await expect(button.getByText(labels.note)).toBeVisible();
        await entranceFinished(page);
        await fitCopyWindow(page);
        await page.mouse.move(0, 0);
        await page.screenshot({
          path: `${outputDir}/overlay-copy-${locale}-${theme}-copy.png`,
        });

        await button.click();
        await expect(
          page.getByRole("button", { name: labels.copied, exact: true }),
        ).toBeVisible();
        await fitCopyWindow(page);
        await page.mouse.move(0, 0);
        await page.screenshot({
          path: `${outputDir}/overlay-copy-${locale}-${theme}-copied.png`,
        });
      });
    }
  }
});
