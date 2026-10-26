import { expect, test, type Page } from "@playwright/test";

const openWindows = async (page: Page) => {
  await page.goto("/tests/fixtures/settings-pages.html?platform=windows");
  await expect(page.getByRole("navigation")).toBeVisible();
};

const nav = (page: Page, name: string) =>
  page.getByRole("navigation").getByRole("button", { name, exact: true });

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
    await expect(page.getByText("Context from the active window")).toHaveCount(
      0,
    );
    // The local engine is named after the PC, never a Mac.
    await expect(page.getByText("On this PC", { exact: true })).toBeVisible();
    await expect(page.getByText(/\bMac\b/)).toHaveCount(0);

    await nav(page, "Advanced").click();
    await expect(page.getByText("Show what's new")).toHaveCount(0);
    await expect(page.getByText("Meetings", { exact: true })).toHaveCount(0);
    await expect(page.getByRole("dialog")).toHaveCount(0);
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
    // Scrolling to an off-screen item must bring it inside its scrollport,
    // not just inside an ancestor whose overflow clips the button.
    for (const section of ["About", "Debug", "General", "Advanced"]) {
      const button = nav(page, section);
      await button.scrollIntoViewIfNeeded();
      await expect(button).toBeInViewport({ ratio: 1 });
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
    await expect(lastSetting).toBeInViewport({ ratio: 1 });
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
