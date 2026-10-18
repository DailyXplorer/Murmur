import { expect, test, type Page } from "@playwright/test";

const outputDir = process.env.MURMUR_SCREENSHOT_DIR;
const VIEWPORT = { width: 960, height: 720 };

const SECTIONS: { file: string; label: string; query?: string }[] = [
  { file: "general", label: "General" },
  { file: "transcription", label: "Transcription" },
  { file: "history", label: "History" },
  { file: "meetings", label: "Meetings", query: "?meetings=1" },
  { file: "insights", label: "Insights" },
  { file: "advanced", label: "Advanced" },
  { file: "debug", label: "Debug" },
  { file: "about", label: "About" },
];

const growViewportToContent = async (page: Page) => {
  const overflow = await page.evaluate(() =>
    Math.max(
      0,
      ...Array.from(document.querySelectorAll<HTMLElement>("*")).map(
        (element) => element.scrollHeight - element.clientHeight,
      ),
    ),
  );
  const { width, height } = page.viewportSize() ?? VIEWPORT;
  await page.setViewportSize({ width, height: height + overflow });
};

test.describe("settings screenshots", () => {
  test.skip(!outputDir, "Set MURMUR_SCREENSHOT_DIR to capture screenshots");

  for (const section of SECTIONS) {
    test(section.file, async ({ page }) => {
      await page.setViewportSize(VIEWPORT);
      await page.goto(
        `/tests/fixtures/settings-pages.html${section.query ?? ""}`,
      );
      await page
        .getByRole("navigation")
        .getByRole("button", { name: section.label, exact: true })
        .click();
      await page.waitForTimeout(500);
      await growViewportToContent(page);
      await page.screenshot({
        path: `${outputDir}/${section.file}.png`,
        fullPage: true,
      });
      expect(
        await page.evaluate(() => window.settingsPages.unhandledCommands),
      ).toEqual([]);
    });
  }

  for (const theme of ["light", "dark"] as const) {
    test(`meetings list and settings (${theme})`, async ({ page }) => {
      await page.emulateMedia({ colorScheme: theme });
      await page.setViewportSize(VIEWPORT);
      await page.goto(`/tests/fixtures/settings-pages.html?theme=${theme}`);
      await page
        .getByRole("navigation")
        .getByRole("button", { name: "Meetings", exact: true })
        .click();
      await page.waitForTimeout(500);
      await growViewportToContent(page);
      await page.screenshot({
        path: `${outputDir}/meetings-list-${theme}.png`,
        fullPage: true,
      });

      await page.setViewportSize(VIEWPORT);
      await page.goto(
        `/tests/fixtures/settings-pages.html?systemAudioSilent=1&theme=${theme}`,
      );
      await page
        .getByRole("navigation")
        .getByRole("button", { name: "Meetings", exact: true })
        .click();
      await page.getByRole("button", { name: /Point hebdo produit/ }).click();
      await page.waitForTimeout(500);
      await page.screenshot({
        path: `${outputDir}/meeting-system-audio-silent-${theme}.png`,
      });
      expect(
        await page.evaluate(() => window.settingsPages.unhandledCommands),
      ).toEqual([]);
    });

    test(`meeting transcript (${theme})`, async ({ page }) => {
      await page.emulateMedia({ colorScheme: theme });
      await page.setViewportSize(VIEWPORT);
      await page.goto(
        `/tests/fixtures/settings-pages.html?meetings=1&meetingFailed=1&theme=${theme}`,
      );
      await page
        .getByRole("navigation")
        .getByRole("button", { name: "Meetings", exact: true })
        .click();
      await page.getByRole("button", { name: /Point hebdo produit/ }).click();
      await page.waitForTimeout(500);
      await growViewportToContent(page);
      await page.screenshot({
        path: `${outputDir}/meeting-transcript-${theme}.png`,
        fullPage: true,
      });

      await page.getByRole("button", { name: "Search Transcript" }).click();
      await page
        .getByRole("textbox", { name: "Search Transcript" })
        .fill("plateforme");
      await page.screenshot({
        path: `${outputDir}/meeting-search-${theme}.png`,
        fullPage: true,
      });

      await page.getByRole("button", { name: "Rename Speakers" }).click();
      await page.waitForTimeout(300);
      await page.screenshot({
        path: `${outputDir}/meeting-rename-${theme}.png`,
      });
      expect(
        await page.evaluate(() => window.settingsPages.unhandledCommands),
      ).toEqual([]);
    });
  }

  for (const theme of ["light", "dark"] as const) {
    test(`meeting summary (${theme})`, async ({ page }) => {
      await page.emulateMedia({ colorScheme: theme });
      await page.setViewportSize(VIEWPORT);
      await page.goto(
        `/tests/fixtures/settings-pages.html?meetings=1&theme=${theme}`,
      );
      await page
        .getByRole("navigation")
        .getByRole("button", { name: "Meetings", exact: true })
        .click();
      await page.getByRole("button", { name: /Point hebdo produit/ }).click();
      await page.waitForTimeout(500);
      await growViewportToContent(page);
      await page.screenshot({
        path: `${outputDir}/meeting-summary-${theme}.png`,
        fullPage: true,
      });

      await page.goto(
        `/tests/fixtures/settings-pages.html?meetings=1&summary=failed&theme=${theme}`,
      );
      await page
        .getByRole("navigation")
        .getByRole("button", { name: "Meetings", exact: true })
        .click();
      await page.getByRole("button", { name: /Point hebdo produit/ }).click();
      await page.waitForTimeout(500);
      await page.screenshot({
        path: `${outputDir}/meeting-summary-failed-${theme}.png`,
      });
      expect(
        await page.evaluate(() => window.settingsPages.unhandledCommands),
      ).toEqual([]);
    });
  }

  // The settings window opens at 680×570.
  for (const models of ["missing", "downloading", "ready"] as const) {
    test(`meetings page at the default window size (models ${models})`, async ({
      page,
    }) => {
      await page.setViewportSize({ width: 680, height: 570 });
      await page.goto(`/tests/fixtures/settings-pages.html?models=${models}`);
      await page
        .getByRole("navigation")
        .getByRole("button", { name: "Meetings", exact: true })
        .click();
      await page.waitForTimeout(500);
      await growViewportToContent(page);
      await page.screenshot({
        path: `${outputDir}/meetings-models-${models}.png`,
        fullPage: true,
      });
    });
  }

  test.describe("buttons at the default window size", () => {
    test.use({
      viewport: { width: 680, height: 570 },
      deviceScaleFactor: 2,
      colorScheme: "light",
    });

    const openMeetingAt = async (page: Page, query: string) => {
      await page.goto(`/tests/fixtures/settings-pages.html?meetings=1${query}`);
      await page
        .getByRole("navigation")
        .getByRole("button", { name: "Meetings", exact: true })
        .click();
      await page.getByRole("button", { name: /Point hebdo produit/ }).click();
      await page.waitForTimeout(500);
    };

    test("meeting views", async ({ page }) => {
      await openMeetingAt(page, "");
      await page.screenshot({ path: `${outputDir}/meeting-summary.png` });

      await openMeetingAt(
        page,
        "&systemAudioSilent=1&systemAudioPermission=granted",
      );
      await page.screenshot({
        path: `${outputDir}/meeting-silent-granted.png`,
      });

      await openMeetingAt(
        page,
        "&systemAudioSilent=1&systemAudioPermission=denied",
      );
      await page.screenshot({
        path: `${outputDir}/meeting-silent-denied.png`,
      });

      await openMeetingAt(page, "&audioBytes=350623");
      await page.getByRole("tab", { name: "Transcript" }).click();
      await page.waitForTimeout(300);
      await page.screenshot({ path: `${outputDir}/meeting-transcript.png` });

      await page
        .getByRole("button", { name: "Delete Audio" })
        .scrollIntoViewIfNeeded();
      await page.waitForTimeout(300);
      await page.screenshot({ path: `${outputDir}/meeting-footer.png` });
    });

    for (const section of ["Transcription", "Meetings", "Advanced", "About"]) {
      test(`${section} page`, async ({ page }) => {
        await page.goto("/tests/fixtures/settings-pages.html?meetings=1");
        await page
          .getByRole("navigation")
          .getByRole("button", { name: section, exact: true })
          .click();
        await page.waitForTimeout(500);
        await growViewportToContent(page);
        await page.screenshot({
          path: `${outputDir}/page-${section.toLowerCase()}.png`,
          fullPage: true,
        });
      });
    }
  });

  test("onboarding", async ({ page }) => {
    await page.setViewportSize(VIEWPORT);
    await page.goto("/tests/fixtures/settings-pages.html?onboarding=1");
    await page.waitForTimeout(500);
    await page.screenshot({
      path: `${outputDir}/onboarding.png`,
      fullPage: true,
    });
  });
});
