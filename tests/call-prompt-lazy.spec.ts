import { expect, test } from "@playwright/test";
import { readFileSync } from "node:fs";
import { entranceFinished, fitCopyWindow, openOverlay } from "./overlayHarness";

const outputDir = process.env.MURMUR_SCREENSHOT_DIR;

// The overlay window is transparent; these stand in for the desktop behind it.
const DESKTOP = { light: "rgb(236 236 240)", dark: "rgb(30 30 34)" };

const tray = (
  JSON.parse(
    readFileSync(
      new URL("../src/i18n/locales/en/translation.json", import.meta.url),
      "utf8",
    ),
  ) as { tray: Record<string, string> }
).tray;

// Rust builds the panel when a call raises the prompt, so the labels are
// stored before the page exists and never reach it as an event.
const RAISED = {
  app: "Zoom",
  kind: "start",
  question: tray.callRecordQuestion,
  accept: tray.callRecord,
  decline: tray.callNotNow,
  position: "bottom",
};

const ENDED = {
  app: "Zoom",
  kind: "ended",
  question: tray.callEndedQuestion.replace("{{app}}", "Zoom"),
  accept: tray.callEndedStop,
  decline: tray.callEndedKeep,
  position: "bottom",
};

test("a panel built for a raised prompt shows it without an event", async ({
  page,
}) => {
  await openOverlay(page, {
    fixture: "call-prompt",
    readyOn: null,
    callPrompt: RAISED,
  });

  const prompt = page.getByRole("alertdialog", { name: RAISED.question });
  await expect(prompt).toBeVisible();
  await expect(prompt).toContainText(RAISED.app);

  await prompt.getByRole("button", { name: RAISED.accept }).click();
  expect(
    await page.evaluate(() => window.overlayHarness.callAnswers()),
  ).toEqual(["record"]);
});

test("a panel built with no prompt raised stays empty", async ({ page }) => {
  await openOverlay(page, { fixture: "call-prompt", readyOn: null });
  await page.waitForTimeout(300);

  await expect(page.getByRole("alertdialog")).toHaveCount(0);
});

test.describe("call prompt built on show, screenshots", () => {
  test.skip(!outputDir, "Set MURMUR_SCREENSHOT_DIR to capture screenshots");
  test.use({ deviceScaleFactor: 2 });

  const prompts = [
    { shot: "call-prompt", labels: RAISED },
    { shot: "call-prompt-ended", labels: ENDED },
  ];
  for (const { shot, labels } of prompts) {
    for (const theme of ["light", "dark"] as const) {
      test(`${shot} ${theme}`, async ({ page }) => {
        await page.emulateMedia({ colorScheme: theme });
        await openOverlay(page, {
          fixture: "call-prompt",
          readyOn: null,
          callPrompt: labels,
        });
        await page.evaluate(
          (background) => (document.body.style.background = background),
          DESKTOP[theme],
        );
        await page.evaluate(() => document.fonts.ready);
        await expect(
          page.getByRole("alertdialog", { name: labels.question }),
        ).toBeVisible();
        await entranceFinished(page);
        await fitCopyWindow(page);
        await page.mouse.move(0, 0);
        await page.screenshot({
          path: `${outputDir}/${shot}-${theme}.png`,
        });
      });
    }
  }
});
