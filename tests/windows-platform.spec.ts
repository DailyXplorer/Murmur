import { expect, test, type Page } from "@playwright/test";

const openWindows = async (page: Page) => {
  await page.goto("/tests/fixtures/settings-pages.html?platform=windows");
  await expect(page.getByRole("navigation")).toBeVisible();
};

const nav = (page: Page, name: string) =>
  page.getByRole("navigation").getByRole("button", { name, exact: true });

const commandsCalled = (page: Page) =>
  page.evaluate(() => window.settingsPages.calls.map((call) => call.command));

const transcribeShortcut = (page: Page) =>
  page.getByRole("button", { name: "Transcribe Shortcut: Press keys..." });

test.describe("Windows build", () => {
  test("offers dictation only", async ({ page }) => {
    await openWindows(page);

    await expect(nav(page, "General")).toBeVisible();
    await expect(nav(page, "Meetings")).toHaveCount(0);
    await expect(page.getByText(/update/i)).toHaveCount(0);

    await nav(page, "Transcription").click();
    await expect(page.getByText("AI Cleanup and Formatting")).toBeVisible();
    await expect(page.getByText("Cleanup Engine")).toHaveCount(0);
    await expect(page.getByText("Transcription service")).toHaveCount(0);
    await expect(page.getByText("Antigravity")).toHaveCount(0);
    await expect(page.getByText("Names From the Active Window")).toHaveCount(0);

    await nav(page, "Advanced").click();
    await expect(page.getByText("Show What's New")).toHaveCount(0);
    await expect(page.getByText("Meetings", { exact: true })).toHaveCount(0);
    await expect(page.getByRole("dialog")).toHaveCount(0);

    const called = await commandsCalled(page);
    expect(called).not.toContain("get_apple_intelligence_status");
    expect(called).not.toContain("get_gemini_status");
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
