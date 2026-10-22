import { expect, test, type Page } from "@playwright/test";
import { DEFAULT_WINDOW } from "./windowSizes";

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

      await page.getByRole("button", { name: "Search transcript" }).click();
      await page
        .getByRole("textbox", { name: "Search transcript" })
        .fill("plateforme");
      await page.screenshot({
        path: `${outputDir}/meeting-search-${theme}.png`,
        fullPage: true,
      });

      await page.getByRole("button", { name: "Rename speakers" }).click();
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

  for (const values of ["normal", "huge"] as const) {
    for (const theme of ["light", "dark"] as const) {
      test(`insights at the default window size (${values}, ${theme})`, async ({
        page,
      }) => {
        await page.emulateMedia({ colorScheme: theme });
        await page.setViewportSize(DEFAULT_WINDOW);
        await page.goto(
          `/tests/fixtures/settings-pages.html?debug=0&insights=${values}&theme=${theme}`,
        );
        await page
          .getByRole("navigation")
          .getByRole("button", { name: "Insights", exact: true })
          .click();
        await page.waitForTimeout(500);
        await page.screenshot({
          path: `${outputDir}/insights-${values}-${theme}.png`,
        });
      });
    }
  }

  for (const models of ["missing", "downloading", "ready"] as const) {
    test(`meetings page at the default window size (models ${models})`, async ({
      page,
    }) => {
      await page.setViewportSize(DEFAULT_WINDOW);
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
      viewport: DEFAULT_WINDOW,
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
        .getByRole("button", { name: "Delete audio" })
        .scrollIntoViewIfNeeded();
      await page.waitForTimeout(300);
      await page.screenshot({ path: `${outputDir}/meeting-footer.png` });
    });

    for (const section of [
      "General",
      "Transcription",
      "Insights",
      "Meetings",
      "Advanced",
      "About",
    ]) {
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

  test.describe("local transcription at the default window size", () => {
    test.use({ viewport: { width: 680, height: 570 }, deviceScaleFactor: 2 });

    const shoot = async (
      page: Page,
      file: string,
      query: string,
      section: string,
    ) => {
      await page.goto(`/tests/fixtures/settings-pages.html?${query}`);
      await page
        .getByRole("navigation")
        .getByRole("button", { name: section, exact: true })
        .click();
      await page.waitForTimeout(400);
      await page.screenshot({ path: `${outputDir}/${file}.png` });
      expect(
        await page.evaluate(() => window.settingsPages.unhandledCommands),
      ).toEqual([]);
    };

    for (const theme of ["light", "dark"] as const) {
      test(`states (${theme})`, async ({ page }) => {
        await page.emulateMedia({ colorScheme: theme });
        const at = `theme=${theme}`;
        for (const models of [
          "missing",
          "downloading",
          "preparing",
          "ready",
          "failed",
          "damaged",
          "noSpace",
        ]) {
          await shoot(
            page,
            `local-${models}-${theme}`,
            `${at}&engine=local&models=${models}`,
            "Transcription",
          );
        }
        await shoot(
          page,
          `local-no-account-${theme}`,
          `${at}&engine=local&account=none`,
          "Transcription",
        );
        await shoot(
          page,
          `local-intel-${theme}`,
          `${at}&engine=local&analysis=unsupported`,
          "Transcription",
        );
        await shoot(
          page,
          `cloud-${theme}`,
          `${at}&engine=cloud`,
          "Transcription",
        );
        await shoot(
          page,
          `cloud-no-account-${theme}`,
          `${at}&engine=cloud&account=none`,
          "Transcription",
        );
        await shoot(
          page,
          `cloud-macos-too-old-${theme}`,
          `${at}&engine=cloud&analysis=os_too_old`,
          "Transcription",
        );
        for (const [query, file] of [
          ["", "delete-with-account"],
          ["&account=none", "delete-without-account"],
          ["&analysis=unsupported", "delete-intel"],
        ]) {
          await shoot(
            page,
            `${file}-${theme}`,
            `${at}&engine=local${query}`,
            "Transcription",
          );
          await page.getByRole("button", { name: "Delete" }).click();
          await page.waitForTimeout(300);
          await page.screenshot({ path: `${outputDir}/${file}-${theme}.png` });
        }
        for (const models of ["missing", "ready", "failed"]) {
          await shoot(
            page,
            `card-${models}-${theme}`,
            `${at}&card=1&models=${models}`,
            "General",
          );
        }
        await shoot(
          page,
          `card-downloading-${theme}`,
          `${at}&card=1&models=missing`,
          "General",
        );
        await page.getByRole("button", { name: /^Download/ }).click();
        await page.waitForTimeout(300);
        await page.screenshot({
          path: `${outputDir}/card-downloading-${theme}.png`,
        });
        await page.evaluate(() => window.settingsPages.finishModelDownload());
        await page.waitForTimeout(300);
        await page.screenshot({
          path: `${outputDir}/card-switched-toast-${theme}.png`,
        });
        await shoot(
          page,
          `history-fallback-${theme}`,
          `${at}&history=fallback`,
          "History",
        );
        await shoot(page, `about-${theme}`, at, "About");
        await page.getByRole("button", { name: "View licences" }).click();
        await page.waitForTimeout(300);
        await page.screenshot({
          path: `${outputDir}/about-licences-${theme}.png`,
        });
        await shoot(
          page,
          `meeting-local-fallback-${theme}`,
          `${at}&meetings=1&localFallback=1`,
          "Meetings",
        );
        await page.getByRole("button", { name: /Point hebdo produit/ }).click();
        await page.waitForTimeout(400);
        await page.screenshot({
          path: `${outputDir}/meeting-local-fallback-${theme}.png`,
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
