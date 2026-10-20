import { expect, test } from "@playwright/test";

test.beforeEach(async ({ page }) => {
  await page.goto("/tests/fixtures/input-change-refused.html");
});

test("a microphone switch refused during a dictation says why", async ({
  page,
}) => {
  const microphone = page.getByTestId("microphone");
  await microphone.getByRole("button", { name: "Built-in Microphone" }).click();
  await page.getByRole("option", { name: "USB Microphone" }).click();

  await expect(
    page.getByText(
      "Couldn't switch the microphone. Stop the dictation in progress, then try again.",
    ),
  ).toBeVisible();
  await expect(
    microphone.getByRole("button", { name: "Built-in Microphone" }),
  ).toBeVisible();
});

test("a microphone reset refused during a dictation says why", async ({
  page,
}) => {
  await page
    .getByTestId("microphone")
    .getByRole("button", { name: /^Reset/ })
    .click();

  await expect(
    page.getByText(
      "Couldn't switch the microphone. Stop the dictation in progress, then try again.",
    ),
  ).toBeVisible();
});

test("a channel switch refused during a dictation says why", async ({
  page,
}) => {
  await page
    .getByTestId("channel")
    .getByRole("button", { name: "Average all channels" })
    .click();
  await page.getByRole("option", { name: "Channel 2" }).click();

  await expect(
    page.getByText(
      "Couldn't switch the input channel. Stop the dictation in progress, then try again.",
    ),
  ).toBeVisible();
});
