import { expect, test, type Locator, type Page } from "@playwright/test";

const openWindows = async (page: Page) => {
  await page.goto("/tests/fixtures/settings-pages.html?platform=windows");
  await expect(page.getByRole("navigation")).toBeVisible();
};

const nav = (page: Page, name: string) =>
  page.getByRole("navigation").getByRole("button", { name, exact: true });

// Scroll offsets/client dimensions are integer CSS pixels while the app's
// 15px root font produces fractional row edges. Allow at most one CSS pixel
// at a scrollport edge, not a percentage that could hide a control's label.
const expectWithinScrollports = async (control: Locator) => {
  await expect(control).toBeVisible();
  await expect
    .poll(() =>
      control.evaluate((element) => {
        const bounds = element.getBoundingClientRect();
        const clip = {
          left: 0,
          top: 0,
          right: innerWidth,
          bottom: innerHeight,
        };
        for (
          let parent = element.parentElement;
          parent;
          parent = parent.parentElement
        ) {
          const style = getComputedStyle(parent);
          const rect = parent.getBoundingClientRect();
          if (/(auto|scroll|hidden|clip)/.test(style.overflowX)) {
            clip.left = Math.max(clip.left, rect.left + parent.clientLeft);
            clip.right = Math.min(
              clip.right,
              rect.left + parent.clientLeft + parent.clientWidth,
            );
          }
          if (/(auto|scroll|hidden|clip)/.test(style.overflowY)) {
            clip.top = Math.max(clip.top, rect.top + parent.clientTop);
            clip.bottom = Math.min(
              clip.bottom,
              rect.top + parent.clientTop + parent.clientHeight,
            );
          }
        }
        return Math.max(
          clip.left - bounds.left,
          bounds.right - clip.right,
          clip.top - bounds.top,
          bounds.bottom - clip.bottom,
          0,
        );
      }),
    )
    .toBeLessThanOrEqual(1);
};

const transcribeShortcut = (page: Page) =>
  page.getByRole("button", { name: "Transcribe Shortcut: Press keys..." });

test.describe("Windows build", () => {
  test("offers dictation only", async ({ page }) => {
    await openWindows(page);

    await expect(nav(page, "General")).toBeVisible();
    await expect(nav(page, "Meetings")).toHaveCount(0);
    await expect(page.getByText(/update/i)).toHaveCount(0);

    await nav(page, "Transcription").click();
    await expect(page.getByText("Cleanup and formatting")).toBeVisible();
    // The local engine is named after the PC, never a Mac.
    await expect(page.getByText("On this PC", { exact: true })).toBeVisible();
    await expect(page.getByText(/\bMac\b/)).toHaveCount(0);

    await nav(page, "Advanced").click();
    await expect(page.getByText("Show what's new")).toHaveCount(0);
    await expect(page.getByText("Meetings", { exact: true })).toHaveCount(0);
    await expect(page.getByRole("dialog")).toHaveCount(0);
  });

  test("offers context from the active window, read through UI Automation", async ({
    page,
  }) => {
    await openWindows(page);
    await nav(page, "Transcription").click();

    const context = page.getByRole("checkbox", {
      name: "Context from the active window",
    });
    await expect(context).toBeChecked();
    await expect(
      page.getByRole("button", {
        name: /^When you start dictating, Murmur reads the text of the active window through Windows accessibility \(UI Automation\)/,
      }),
    ).toBeVisible();
    await expect(page.getByText(/through Accessibility/)).toHaveCount(0);

    await context.uncheck({ force: true });
    await expect
      .poll(() =>
        page.evaluate(() =>
          window.settingsPages.calls
            .filter((call) => call.command === "change_window_names_setting")
            .map((call) => call.args),
        ),
      )
      .toEqual([{ enabled: false }]);
  });

  test("previews no macOS release notes on the Debug page", async ({
    page,
  }) => {
    await openWindows(page);
    await nav(page, "Debug").click();

    await expect(page.getByText("Log level", { exact: true })).toBeVisible();
    await expect(page.getByText("Preview what's new")).toHaveCount(0);
  });

  test("credits only the speech model it downloads", async ({ page }) => {
    await openWindows(page);
    await nav(page, "About").click();
    await page.getByRole("button", { name: "View licences" }).click();

    const dialog = page.getByRole("dialog");
    await expect(dialog.getByRole("listitem")).toHaveCount(1);
    await expect(dialog).toContainText("Parakeet TDT 0.6B v3 by NVIDIA");
    await expect(dialog).not.toContainText("Core ML");
  });

  test("names the Ctrl+Win default with Windows keys", async ({ page }) => {
    await openWindows(page);
    await expect(transcribeShortcut(page)).toHaveText("Ctrl + Win");
  });

  test("records a chord of modifiers alone once they are released", async ({
    page,
  }) => {
    await openWindows(page);
    await transcribeShortcut(page).click();

    await page.keyboard.down("Control");
    await page.keyboard.down("Meta");
    await page.keyboard.up("Meta");
    await page.keyboard.up("Control");

    // The fixture rejects the write, so only the first call is the capture.
    await expect
      .poll(() =>
        page.evaluate(
          () =>
            window.settingsPages.calls.find(
              (call) => call.command === "change_binding",
            )?.args,
        ),
      )
      .toEqual({ id: "transcribe", binding: "ctrl+command" });
  });

  test("records and shows a shortcut as an AZERTY keyboard prints it", async ({
    page,
  }) => {
    await page.goto(
      "/tests/fixtures/settings-pages.html?platform=windows&keyboard=azerty",
    );
    const pasteLast = page.getByRole("button", {
      name: "Paste last transcription: Press keys...",
    });

    // Physical key (its US name), the shortcut saved, the row shown.
    for (const [code, binding, shown] of [
      // Windows fires "ctrl+1" on the key printed "&".
      ["Digit1", "ctrl+1", "Ctrl + &"],
      // The key a US keyboard calls Q is the A key.
      ["KeyQ", "ctrl+a", "Ctrl + A"],
      ["KeyM", "ctrl+,", "Ctrl + ,"],
      ["Semicolon", "ctrl+m", "Ctrl + M"],
      ["Period", "ctrl+/", "Ctrl + :"],
    ]) {
      await pasteLast.click();
      await page.keyboard.press(`Control+${code}`);

      await expect(pasteLast).toHaveText(shown);
      await expect
        .poll(() =>
          page.evaluate(() => {
            const changes = window.settingsPages.calls.filter(
              (call) => call.command === "change_binding",
            );
            return changes[changes.length - 1]?.args;
          }),
        )
        .toEqual({ id: "paste_last", binding });
    }
  });
});

// Logical client sizes derived from physical work areas and native borders.
// These are browser layout checks, not native Windows DPI tests.
for (const viewport of [
  { width: 780, height: 681 }, // 1024x768, 100%
  { width: 780, height: 527 }, // 1366x768, 125%
  { width: 780, height: 425 }, // 1366x768, 150%
  { width: 666, height: 425 }, // 1024x768, 150%
  { width: 666, height: 313 }, // 1024x600, 150%; sidebar must scroll
]) {
  test(`small Windows settings remain reachable at ${viewport.width}x${viewport.height}`, async ({
    page,
  }) => {
    await page.setViewportSize(viewport);
    await openWindows(page);
    await expect
      .poll(() =>
        page
          .getByRole("navigation")
          .evaluate((element) => element.scrollWidth <= element.clientWidth),
      )
      .toBe(true);
    // Scrolling to an off-screen item must bring it inside its scrollport,
    // not just inside an ancestor whose overflow clips the button.
    for (const section of ["About", "Debug", "General", "Advanced"]) {
      const button = nav(page, section);
      await button.scrollIntoViewIfNeeded();
      await expectWithinScrollports(button);
      await expect(button.locator("span")).toBeInViewport({ ratio: 1 });
      if (section === "About" && viewport.height === 313) {
        expect(
          await page
            .getByRole("navigation")
            .evaluate((element) => element.scrollTop),
        ).toBeGreaterThan(0);
      }
      await button.click();
      await expect(button).toHaveAttribute("aria-current", "page");
    }
    const lastSetting = page
      .getByRole("main", { name: "Advanced" })
      .getByRole("checkbox")
      .last()
      .locator("..");
    await lastSetting.scrollIntoViewIfNeeded();
    await expectWithinScrollports(lastSetting);
    await lastSetting.click();
    await expect(page.getByText("v1.3.7", { exact: true })).toBeInViewport({
      ratio: 1,
    });
    expect(
      await page.evaluate(
        () => document.documentElement.scrollWidth <= window.innerWidth,
      ),
    ).toBe(true);
  });
}
