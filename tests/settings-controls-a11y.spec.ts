import { expect, test } from "@playwright/test";

test("settings controls expose names and keyboard-operable roles", async ({
  page,
}) => {
  await page.goto("/tests/fixtures/production-settings-control-widths.html");

  const shortcutButton = page.getByRole("button", {
    name: "Transcribe Shortcut: Press keys...",
  });
  await expect(shortcutButton).toBeVisible();
  await expect(page.getByRole("button", { name: "Reset" })).toHaveCount(3);
  const keyboardActivation = page.evaluate(
    () =>
      new Promise<boolean>((resolve) => {
        const button = document.querySelector<HTMLButtonElement>(
          'button[aria-label="Transcribe Shortcut: Press keys..."]',
        );
        button?.addEventListener("click", () => resolve(true), { once: true });
      }),
  );
  await shortcutButton.focus();
  await page.keyboard.press("Enter");
  await expect(keyboardActivation).resolves.toBe(true);

  await page.goto("/tests/fixtures/accent-controls.html");
  await expect(
    page.getByRole("slider", { name: "Overlay size" }),
  ).toBeVisible();
});
