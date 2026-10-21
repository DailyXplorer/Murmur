import { expect, test } from "@playwright/test";
import { readFileSync } from "node:fs";
import {
  COPY_WINDOW,
  dispatch,
  entranceFinished,
  openOverlay,
} from "./overlayHarness";

const outputDir = process.env.MURMUR_SCREENSHOT_DIR;

// The overlay window is transparent; these stand in for the desktop behind it.
const DESKTOP = { light: "rgb(236 236 240)", dark: "rgb(30 30 34)" };

const trayLabels = (locale: string) => {
  const { tray } = JSON.parse(
    readFileSync(
      new URL(
        `../src/i18n/locales/${locale}/translation.json`,
        import.meta.url,
      ),
      "utf8",
    ),
  ) as { tray: Record<string, string> };
  return { copy: tray.overlayCopy, copied: tray.overlayCopied };
};

test.describe("copy pill screenshots", () => {
  test.skip(!outputDir, "Set MURMUR_SCREENSHOT_DIR to capture screenshots");
  test.use({ viewport: COPY_WINDOW, deviceScaleFactor: 2 });

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
        const labels = trayLabels(locale);

        await dispatch(page, "show-overlay-copy", labels);
        const button = page.getByRole("button", { name: labels.copy });
        await expect(button).toBeVisible();
        await entranceFinished(page);
        await page.mouse.move(0, 0);
        await page.screenshot({
          path: `${outputDir}/overlay-copy-${locale}-${theme}-copy.png`,
        });

        await button.click();
        await expect(
          page.getByRole("button", { name: labels.copied }),
        ).toBeVisible();
        await page.mouse.move(0, 0);
        await page.screenshot({
          path: `${outputDir}/overlay-copy-${locale}-${theme}-copied.png`,
        });
      });
    }
  }
});
