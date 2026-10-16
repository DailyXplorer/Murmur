import { expect, test } from "@playwright/test";

test("a denied microphone leads to its System Settings pane", async ({
  page,
}) => {
  await page.goto("/tests/fixtures/microphone-denied.html");
  await expect(page.getByTestId("microphone-denied-fixture")).toBeVisible();

  await page.getByRole("button", { name: "Grant Permission" }).click();

  await expect(
    page.getByText(
      "Grant microphone access in System Settings → Privacy & Security → Microphone.",
    ),
  ).toBeVisible();
  await page.getByRole("button", { name: "Open System Settings" }).click();
  await expect
    .poll(() =>
      page.evaluate(() =>
        window.microphoneDeniedFixture.microphoneSettingsOpened(),
      ),
    )
    .toBe(1);

  await page.evaluate(() => window.microphoneDeniedFixture.grantMicrophone());
  await expect(page.getByText("All set!")).toBeVisible();
});
