import { expect, test, type Page } from "@playwright/test";

const field = (page: Page, id: string) =>
  page
    .getByTestId(`shortcut-${id}`)
    .getByRole("button", { name: /: Press keys\.\.\.$/ });

const capture = (page: Page, id: string) =>
  page.getByTestId(`shortcut-${id}`).getByRole("status");

const calls = (page: Page) =>
  page.evaluate(() => window.shortcutCapture.calls());

test.beforeEach(async ({ page }) => {
  await page.goto("/tests/fixtures/shortcut-capture.html");
  await expect(field(page, "transcribe")).toHaveText("Option + Space");
});

const pressCombination = async (page: Page, id: string, key: string) => {
  await page.keyboard.down(key);
  await expect(capture(page, id)).toHaveText(key);
  await page.keyboard.up(key);
};

test("opening a second shortcut field moves the capture to it", async ({
  page,
}) => {
  await field(page, "transcribe").click();
  await expect(capture(page, "transcribe")).toBeVisible();
  await field(page, "cancel").click();
  await expect(capture(page, "cancel")).toBeVisible();
  await expect(capture(page, "transcribe")).toHaveCount(0);

  await pressCombination(page, "cancel", "F8");

  await expect(field(page, "cancel")).toHaveText("F8");
  await expect(field(page, "transcribe")).toHaveText("Option + Space");
  await expect
    .poll(() => calls(page))
    .toEqual([
      "suspend_all_bindings",
      "change_binding cancel f8",
      "resume_all_bindings",
    ]);
});

const leaveWindow = {
  "switching to another app": () => window.dispatchEvent(new Event("blur")),
  "hiding the Settings window": () => {
    Object.defineProperty(document, "visibilityState", {
      configurable: true,
      value: "hidden",
    });
    document.dispatchEvent(new Event("visibilitychange"));
  },
  "leaving the settings page": () => window.shortcutCapture.unmount(),
};

for (const [action, leave] of Object.entries(leaveWindow)) {
  test(`${action} while capturing turns the shortcuts back on`, async ({
    page,
  }) => {
    await field(page, "transcribe").click();
    await expect(capture(page, "transcribe")).toBeVisible();
    await expect.poll(() => calls(page)).toEqual(["suspend_all_bindings"]);

    await page.evaluate(leave);

    await expect(capture(page, "transcribe")).toHaveCount(0);
    await expect
      .poll(() => calls(page))
      .toEqual(["suspend_all_bindings", "resume_all_bindings"]);
  });
}

test("keys are sent in the spelling the shortcut parser accepts", async ({
  page,
}) => {
  await field(page, "cancel").click();
  await page.keyboard.press("PageUp");
  await expect(field(page, "cancel")).toHaveText("Page Up");

  await field(page, "cancel").click();
  await page.keyboard.press("NumpadAdd");
  await expect(field(page, "cancel")).toHaveText("Numpad +");

  await expect
    .poll(() => calls(page))
    .toEqual([
      "suspend_all_bindings",
      "change_binding cancel pageup",
      "resume_all_bindings",
      "suspend_all_bindings",
      "change_binding cancel numpadadd",
      "resume_all_bindings",
    ]);
});

test("a bare Escape abandons the edit instead of binding Escape", async ({
  page,
}) => {
  await field(page, "transcribe").click();
  await expect(capture(page, "transcribe")).toBeVisible();

  await page.keyboard.press("Escape");

  await expect(field(page, "transcribe")).toHaveText("Option + Space");
  await expect
    .poll(() => calls(page))
    .toEqual(["suspend_all_bindings", "resume_all_bindings"]);
});
