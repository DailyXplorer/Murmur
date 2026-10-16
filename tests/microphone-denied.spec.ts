import { expect, test } from "@playwright/test";

const DENIED_HINT =
  "Grant microphone access in System Settings → Privacy & Security → Microphone.";

test("a denied microphone leads to its System Settings pane", async ({
  page,
}) => {
  await page.goto("/tests/fixtures/microphone-denied.html?microphone=2");
  await expect(page.getByTestId("microphone-denied-fixture")).toBeVisible();

  await expect(page.getByText(DENIED_HINT)).toBeVisible();
  await page.getByRole("button", { name: "Open System Settings" }).click();
  await expect
    .poll(() =>
      page.evaluate(() =>
        window.microphoneDeniedFixture.microphoneSettingsOpened(),
      ),
    )
    .toBe(1);

  await page.evaluate(() =>
    window.microphoneDeniedFixture.setMicrophoneStatus(3),
  );
  await expect(page.getByText("All set!")).toBeVisible();
});

test("the open system prompt shows no denied hint until the user denies", async ({
  page,
}) => {
  await page.goto("/tests/fixtures/microphone-denied.html?microphone=0");
  await expect(page.getByTestId("microphone-denied-fixture")).toBeVisible();

  await page.getByRole("button", { name: "Grant Permission" }).click();

  await expect(page.getByText("Waiting...")).toBeVisible();
  await expect(page.getByText(DENIED_HINT)).toBeHidden();
  await expect(
    page.getByRole("button", { name: "Open System Settings" }),
  ).toBeHidden();

  await page.evaluate(() =>
    window.microphoneDeniedFixture.setMicrophoneStatus(2),
  );
  await expect(page.getByText(DENIED_HINT)).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Open System Settings" }),
  ).toBeVisible();
});
