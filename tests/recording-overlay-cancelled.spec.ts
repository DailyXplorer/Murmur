import { expect, test } from "@playwright/test";
import { readFileSync } from "node:fs";
import {
  dispatch,
  entranceFinished,
  fitCopyWindow,
  openOverlay,
} from "./overlayHarness";

const outputDir = process.env.MURMUR_SCREENSHOT_DIR;

// The overlay window is transparent; these stand in for the desktop behind it.
const DESKTOP = { light: "rgb(236 236 240)", dark: "rgb(30 30 34)" };

const labels = (locale: string) => {
  const { tray } = JSON.parse(
    readFileSync(
      new URL(
        `../src/i18n/locales/${locale}/translation.json`,
        import.meta.url,
      ),
      "utf8",
    ),
  ) as { tray: Record<string, string> };
  return { note: tray.overlayCancelled, restore: tray.overlayRestore };
};

test("the cancelled pill restores the dictation once per click", async ({
  page,
}) => {
  await openOverlay(page);
  const english = labels("en");

  await dispatch(page, "show-overlay-cancelled", english);
  const restore = page.getByRole("button", {
    name: english.restore,
    exact: true,
  });
  await expect(restore).toBeVisible();
  await expect(restore).toHaveAccessibleDescription(english.note);
  await expect
    .poll(() => page.evaluate(() => window.overlayHarness.copyFits().length))
    .toBeGreaterThan(0);

  await restore.click();
  await expect
    .poll(() => page.evaluate(() => window.overlayHarness.restoreCalls()))
    .toBe(1);
});

test("the cancelled pill opens from the orb to its note and Restore", async ({
  page,
}) => {
  await openOverlay(page);
  await page.evaluate(() => document.fonts.ready);
  const english = labels("en");
  const widths = await page.evaluate(
    (shown) =>
      new Promise<number[]>((resolve) => {
        window.overlayHarness.dispatch("show-overlay-cancelled", shown);
        const seen: number[] = [];
        const start = performance.now();
        requestAnimationFrame(function sample(now) {
          const pill = document.querySelector<HTMLElement>(".ov-copy");
          if (pill) seen.push(pill.offsetWidth);
          if (now - start < 900) requestAnimationFrame(sample);
          else resolve(seen);
        });
      }),
    english,
  );

  expect(widths.length, "the pill showed").toBeGreaterThan(10);
  expect(widths[0], "width starts at the orb").toBeLessThanOrEqual(40);
  expect(widths[widths.length - 1], "width ends fitted").toBeGreaterThan(150);
  await expect(page.locator(".ov-glyph")).toHaveCount(1);
});

test("Escape's dismissal and a new recording both take the pill away", async ({
  page,
}) => {
  await openOverlay(page);
  const english = labels("en");
  const restore = page.getByRole("button", { name: english.restore });

  // The backend's Escape monitor dismisses a fitted pill with hide-overlay.
  await dispatch(page, "show-overlay-cancelled", english);
  await expect(restore).toBeVisible();
  await dispatch(page, "hide-overlay");
  await expect(restore).toHaveCount(0);

  await dispatch(page, "show-overlay-cancelled", english);
  await expect(restore).toBeVisible();
  await dispatch(page, "show-overlay", "recording");
  await expect(restore).toHaveCount(0);
  await expect(page.locator(".ov-capsule.arming")).toBeVisible();
  expect(await page.evaluate(() => window.overlayHarness.restoreCalls())).toBe(
    0,
  );
});

test.describe("cancelled pill screenshots", () => {
  test.skip(!outputDir, "Set MURMUR_SCREENSHOT_DIR to capture screenshots");
  test.use({ deviceScaleFactor: 2 });

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
        const strings = labels(locale);

        await dispatch(page, "show-overlay-cancelled", strings);
        await expect(
          page.getByRole("button", { name: strings.restore, exact: true }),
        ).toBeVisible();
        await entranceFinished(page);
        await fitCopyWindow(page);
        await page.mouse.move(0, 0);
        await page.screenshot({
          path: `${outputDir}/overlay-cancelled-${locale}-${theme}.png`,
        });
      });
    }
  }
});
