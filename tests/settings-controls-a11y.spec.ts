import { expect, test } from "@playwright/test";

test("settings controls expose names and keyboard-operable roles", async ({
  page,
}) => {
  await page.goto("/tests/fixtures/production-settings-control-widths.html");

  const shortcutButton = () =>
    page.getByRole("button", {
      name: "Transcribe Shortcut: Press keys...",
    });
  const shortcutStatus = page.getByRole("status");

  await expect(shortcutButton()).toBeVisible();
  await expect(page.getByRole("button", { name: "Reset" })).toHaveCount(3);

  const expectEditorToOpenAndClose = async (activate: () => Promise<void>) => {
    await activate();
    await expect(shortcutStatus).toHaveText("Press keys...");

    await page.mouse.click(10, 10);
    await expect(shortcutStatus).toHaveCount(0);
  };

  await expectEditorToOpenAndClose(() => shortcutButton().click());
  await expectEditorToOpenAndClose(async () => {
    await shortcutButton().focus();
    await page.keyboard.press("Enter");
  });
  await expectEditorToOpenAndClose(async () => {
    await shortcutButton().focus();
    await page.keyboard.press("Space");
  });

  await page.goto("/tests/fixtures/accent-controls.html");
  await expect(
    page.getByRole("slider", { name: "Overlay size" }),
  ).toBeVisible();
});
