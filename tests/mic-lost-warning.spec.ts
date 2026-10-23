import { expect, test, type Locator, type Page } from "@playwright/test";
import { readdirSync, readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import {
  dispatch,
  entranceFinished,
  fitCopyWindow,
  openOverlay,
} from "./overlayHarness";

const localesDir = fileURLToPath(
  new URL("../src/i18n/locales/", import.meta.url),
);
const trayStringsByLocale = readdirSync(localesDir).map((locale) => {
  const { tray } = JSON.parse(
    readFileSync(`${localesDir}${locale}/translation.json`, "utf8"),
  ) as { tray: Record<string, string> };
  return { locale, tray };
});
const english = trayStringsByLocale.find(({ locale }) => locale === "en")!.tray;

const WARNING = english.overlayMicDisconnected;
const WARNED_COPY = {
  copy: english.overlayCopy,
  copied: english.overlayCopied,
  note: english.overlayNotPasted,
  warning: WARNING,
};

// As in recording-overlay-state.spec.ts: wider than any fallback font.
const WIDE_FONT = `.ov-stage {
  font-family: Verdana, "DejaVu Sans", sans-serif !important;
  letter-spacing: 0.08em;
}`;
const WIDEST_PILL = 560;

const copyCalls = (page: Page) =>
  page.evaluate(() => window.overlayHarness.copyCalls());

/** Where a fitted pill's text and action sit in it, and in the window. */
const layout = (pill: Locator) =>
  pill.evaluate((capsule: HTMLElement) => {
    const box = capsule.getBoundingClientRect();
    const lineRects = (text: HTMLElement) => {
      const range = document.createRange();
      range.selectNodeContents(text);
      return [...range.getClientRects()];
    };
    const texts = [...capsule.querySelectorAll<HTMLElement>(".ov-copy-note")];
    const lines = texts.flatMap(lineRects);
    const textLeft = Math.min(...lines.map((line) => line.left));
    const textRight = Math.max(...lines.map((line) => line.right));
    const action = capsule
      .querySelector(".ov-copy-action")
      ?.getBoundingClientRect();
    return {
      width: capsule.offsetWidth,
      lineCounts: texts.map(
        (text) =>
          new Set(lineRects(text).map((line) => Math.round(line.top))).size,
      ),
      clipped:
        capsule.scrollWidth > capsule.clientWidth ||
        capsule.scrollHeight > capsule.clientHeight ||
        lines.some(
          (line) =>
            line.left < box.left ||
            line.right > box.right ||
            line.top < box.top ||
            line.bottom > box.bottom,
        ),
      insideWindow:
        box.left >= 0 &&
        box.right <= window.innerWidth &&
        box.top >= 0 &&
        box.bottom <= window.innerHeight,
      startInset: textLeft - box.left,
      endInset: box.right - (action ? action.right : textRight),
      gapToAction: action ? action.left - textRight : null,
    };
  });

test("a microphone lost mid-dictation shows its one warning sentence in a fitted pill", async ({
  page,
}) => {
  await openOverlay(page);
  await dispatch(page, "show-overlay", "transcribing");
  await dispatch(page, "show-overlay-warning", WARNING);

  await expect(page.getByRole("status")).toHaveText(WARNING);
  await expect(page.locator(".ov-stage")).toHaveText(WARNING);
  await expect(page.locator(".ov-stage button")).toHaveCount(0);
  await expect(page.locator(".ov-wave")).toHaveCount(0);
  // Sized like the copy pill, from what the webview measures.
  await fitCopyWindow(page);
});

test("the warning pill grows from the rest capsule to its text", async ({
  page,
}) => {
  await openOverlay(page);
  await page.evaluate(() => document.fonts.ready);
  const widths = await page.evaluate(
    (message) =>
      new Promise<number[]>((resolve) => {
        window.overlayHarness.dispatch("show-overlay-warning", message);
        const seen: number[] = [];
        const start = performance.now();
        requestAnimationFrame(function sample(now) {
          const pill = document.querySelector<HTMLElement>(".ov-warning");
          if (pill) seen.push(pill.offsetWidth);
          if (now - start < 900) requestAnimationFrame(sample);
          else resolve(seen);
        });
      }),
    WARNING,
  );

  expect(widths.length, "the pill showed").toBeGreaterThan(10);
  expect(widths[0], "width starts at rest").toBeLessThanOrEqual(112);
  expect(widths[widths.length - 1], "width ends fitted").toBeGreaterThan(250);
});

test("the copy pill puts the warning above its note and a late event leaves Copy copying the dictation", async ({
  page,
}) => {
  await openOverlay(page);
  await dispatch(page, "show-overlay", "transcribing");
  await dispatch(page, "show-overlay-copy", WARNED_COPY);
  const button = page.getByRole("button", { name: "Copy", exact: true });
  await expect(button).toBeVisible();
  await expect(button).toHaveAccessibleDescription(
    `${WARNING} ${WARNED_COPY.note}`,
  );
  await expect(page.locator(".ov-stage button")).toHaveCount(1);
  await entranceFinished(page);
  const warningBox = await button.getByText(WARNING).boundingBox();
  const noteBox = await button.getByText(WARNED_COPY.note).boundingBox();
  expect(warningBox!.y + warningBox!.height).toBeLessThanOrEqual(noteBox!.y);

  // The cut-short recording's last level and a readiness that came too late.
  expect(await dispatch(page, "mic-level", Array(16).fill(0.9))).toBe(1);
  expect(await dispatch(page, "recording-ready")).toBe(1);
  await expect(button).toBeVisible();
  expect(await copyCalls(page), "nothing copied on its own").toBe(0);

  await button.click();
  await expect(
    page.getByRole("button", { name: "Copied", exact: true }),
  ).toBeVisible();
  expect(await copyCalls(page)).toBe(1);
});

test("a copy pill without the warning looks as before", async ({ page }) => {
  await openOverlay(page);
  await dispatch(page, "show-overlay-copy", { ...WARNED_COPY, warning: null });
  const button = page.getByRole("button", { name: "Copy", exact: true });
  await expect(button).toHaveAccessibleDescription(WARNED_COPY.note);
  await expect(page.getByText(WARNING)).toHaveCount(0);
});

test("the warning pill hides on Escape or its timer and gives way to a new recording", async ({
  page,
}) => {
  await openOverlay(page);
  await dispatch(page, "show-overlay-warning", WARNING);
  await expect(page.getByRole("status")).toBeVisible();
  // Escape, the cancel shortcut's default key, and the timer both end in
  // the backend's hide.
  await dispatch(page, "hide-overlay");
  await expect(page.locator(".ov-stage")).toHaveCount(0);

  await dispatch(page, "show-overlay-warning", WARNING);
  await expect(page.getByRole("status")).toBeVisible();
  await dispatch(page, "show-overlay", "recording");
  await expect(page.getByRole("status")).toHaveCount(0);
  await expect(page.locator(".ov-wave")).toBeVisible();
});

test("reduced motion shows the warning pill without any animation", async ({
  page,
}) => {
  await page.emulateMedia({ reducedMotion: "reduce" });
  await openOverlay(page);
  await dispatch(page, "show-overlay-warning", WARNING);
  await expect(page.getByRole("status")).toBeVisible();
  expect(await page.evaluate(() => document.getAnimations().length)).toBe(0);
});

for (const { locale, tray } of trayStringsByLocale) {
  test(`the ${locale} warning is its own full sentence`, () => {
    const warning = tray.overlayMicDisconnected;
    expect(warning).toMatch(/^\p{Lu}.*\.$/u);
    if (locale !== "en") expect(warning).not.toBe(WARNING);
  });

  for (const font of ["app font", "wide font"] as const) {
    test(`the ${locale} warning fits its pill and the copy pill evenly with the ${font}`, async ({
      page,
    }) => {
      await openOverlay(page);
      if (font === "wide font") await page.addStyleTag({ content: WIDE_FONT });
      await page.evaluate(() => document.fonts.ready);
      const warning = tray.overlayMicDisconnected;
      const shows = [
        ["show-overlay-warning", warning, page.getByRole("status")],
        [
          "show-overlay-copy",
          {
            copy: tray.overlayCopy,
            copied: tray.overlayCopied,
            note: tray.overlayNotPasted,
            warning,
          },
          page.getByRole("button", { name: tray.overlayCopy, exact: true }),
        ],
      ] as const;

      for (const [event, payload, pill] of shows) {
        await page.setViewportSize({ width: 1280, height: 720 });
        await dispatch(page, event, payload);
        await expect(pill).toContainText(warning);
        await entranceFinished(page);
        await fitCopyWindow(page);
        const fit = await layout(pill);
        test.info().annotations.push({
          type: `${event} lines`,
          description: fit.lineCounts.join(", "),
        });

        expect(fit.width, `${event}: pill width`).toBeLessThanOrEqual(
          WIDEST_PILL,
        );
        expect(fit.clipped, `${event}: clipped`).toBe(false);
        expect(fit.insideWindow, `${event}: inside its window`).toBe(true);
        expect(
          Math.max(...fit.lineCounts),
          `${event}: two lines at most`,
        ).toBeLessThanOrEqual(2);
        expect(
          fit.startInset,
          `${event}: text clears the rounded edge`,
        ).toBeGreaterThanOrEqual(12);
        expect(
          Math.abs(fit.endInset - fit.startInset),
          `${event}: as much room at the end (${fit.endInset}px) as at the start (${fit.startInset}px)`,
        ).toBeLessThanOrEqual(1);
        if (fit.gapToAction !== null) {
          expect(
            fit.gapToAction,
            `${event}: text ends by the action`,
          ).toBeGreaterThanOrEqual(0);
          expect(
            fit.gapToAction,
            `${event}: text ends by the action`,
          ).toBeLessThan(12);
        }
        await dispatch(page, "hide-overlay");
      }
    });
  }
}
