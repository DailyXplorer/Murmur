import { expect, test, type Locator } from "@playwright/test";

const LONG_CUSTOM_WORD = "W".repeat(50);
const RAIL_WIDTH = 260;
const WIDTH_TOLERANCE = 1;

const controlSlot = (control: Locator) =>
  control.locator(
    "xpath=ancestor::div[contains(@class, 'settings-control-rail-width')][1]",
  );

const expectWidth = (width: number, expected: number) => {
  expect(Math.abs(width - expected)).toBeLessThanOrEqual(WIDTH_TOLERANCE);
};

test("real settings controls consume the shared rail", async ({ page }) => {
  await page.setViewportSize({ width: 960, height: 1800 });
  await page.goto("/tests/fixtures/production-settings-control-widths.html");

  const microphone = page
    .getByTestId("production-microphone")
    .getByRole("button", { name: "Default" });
  const language = page
    .getByTestId("production-language")
    .getByRole("button", { name: "Auto Detect" });
  const shortcut = page
    .getByTestId("production-shortcut")
    .getByText("Alt + Space", { exact: true });
  const historyLabel = page
    .getByTestId("production-history-limit")
    .getByText("entries", { exact: true });
  const customWordsInput = page
    .getByTestId("production-custom-words")
    .getByPlaceholder("Add a word");
  const provider = page
    .getByTestId("production-transcription")
    .locator('button[aria-haspopup="listbox"]')
    .first();
  const geminiStatus = page
    .getByTestId("production-transcription")
    .getByText("Connected", { exact: true })
    .last();
  const metaApiKey = page
    .getByTestId("production-transcription")
    .getByLabel("Meta API key");

  await expect(geminiStatus).toBeVisible();
  await expect(metaApiKey).toHaveAttribute("type", "password");

  const slots = [
    microphone,
    language,
    shortcut,
    historyLabel,
    customWordsInput,
    provider,
    geminiStatus,
    metaApiKey,
  ].map(controlSlot);
  const slotNames = [
    "microphone",
    "language",
    "shortcut",
    "history limit",
    "custom words",
    "provider",
    "Gemini status",
    "Meta API key",
  ];
  const slotMeasurements = await Promise.all(
    slots.map((slot) =>
      slot.evaluate((element) => {
        const rect = element.getBoundingClientRect();
        return {
          clientWidth: element.clientWidth,
          right: rect.right,
          scrollWidth: element.scrollWidth,
          width: rect.width,
        };
      }),
    ),
  );

  for (const [index, measurement] of slotMeasurements.entries()) {
    expectWidth(measurement.width, RAIL_WIDTH);
    expect(
      measurement.scrollWidth,
      `${slotNames[index]} control should not overflow`,
    ).toBeLessThanOrEqual(measurement.clientWidth);
    expect(
      measurement.right,
      `${slotNames[index]} control should share the rail edge`,
    ).toBeCloseTo(slotMeasurements[0].right, 1);
  }

  const providerBox = await provider.boundingBox();
  expect(providerBox).not.toBeNull();
  expectWidth(providerBox?.width ?? 0, RAIL_WIDTH);

  const compositeControls = [
    {
      end: page.getByTestId("production-microphone").getByRole("button").last(),
      start: microphone,
    },
    {
      end: page.getByTestId("production-language").getByRole("button").last(),
      start: language,
    },
    {
      end: page.getByTestId("production-shortcut").getByRole("button").last(),
      start: shortcut,
    },
    {
      end: page
        .getByTestId("production-custom-words")
        .getByRole("button", { name: "Add" }),
      start: customWordsInput,
    },
    {
      end: page
        .getByTestId("production-transcription")
        .getByRole("button", { name: "Open", exact: true }),
      start: metaApiKey,
    },
  ];
  for (const { end, start } of compositeControls) {
    const [endBox, startBox] = await Promise.all([
      end.boundingBox(),
      start.boundingBox(),
    ]);
    expect(endBox).not.toBeNull();
    expect(startBox).not.toBeNull();
    expect(
      (endBox?.x ?? 0) + (endBox?.width ?? 0) - (startBox?.x ?? 0),
    ).toBeCloseTo(RAIL_WIDTH, 1);
  }

  const historyBox = await historyLabel.boundingBox();
  expect(historyBox).not.toBeNull();
  expect((historyBox?.x ?? 0) + (historyBox?.width ?? 0)).toBeCloseTo(
    slotMeasurements[3].right,
    1,
  );
  expect(
    await shortcut.evaluate((element) => getComputedStyle(element).textAlign),
  ).toBe("start");

  const longChip = page
    .getByTestId("production-custom-words")
    .getByTitle(LONG_CUSTOM_WORD);
  const chipLane = longChip.locator("..");
  const chipMeasurements = await chipLane.evaluate((element) => ({
    clientWidth: element.clientWidth,
    scrollWidth: element.scrollWidth,
    width: element.getBoundingClientRect().width,
    x: element.getBoundingClientRect().x,
  }));
  expect(chipMeasurements.width).toBeGreaterThan(RAIL_WIDTH);
  expect(chipMeasurements.scrollWidth).toBeLessThanOrEqual(
    chipMeasurements.clientWidth,
  );

  const customWordsTitle = page
    .getByTestId("production-custom-words")
    .getByRole("heading", { name: "Custom Words" });
  const customTitleBox = await customWordsTitle.boundingBox();
  expect(customTitleBox).not.toBeNull();
  expect(chipMeasurements.x).toBeCloseTo(customTitleBox?.x ?? 0, 1);

  await metaApiKey.fill("meta-test-secret");
  await page
    .getByTestId("production-transcription")
    .getByRole("button", { name: "Save" })
    .click();
  await expect(
    page
      .getByTestId("production-transcription")
      .getByText("Configured", { exact: true }),
  ).toBeVisible();
  await expect(page.getByText("meta-test-secret")).toHaveCount(0);

  await page
    .getByTestId("production-transcription")
    .getByRole("button", { name: "Delete" })
    .click();
  await expect(
    page.getByTestId("production-transcription").getByLabel("Meta API key"),
  ).toHaveAttribute("type", "password");

  await provider.click();
  await page
    .getByRole("option", { name: "Meta AI app (experimental)" })
    .click();
  await expect(provider).toHaveText("Meta AI app (experimental)");
  await expect(
    page
      .getByTestId("production-transcription")
      .getByText("Direct app dictation", { exact: true }),
  ).toBeVisible();
  await expect(
    page
      .getByTestId("production-transcription")
      .getByRole("heading", { name: "Custom Words" }),
  ).toHaveCount(0);
});

test("Meta AI launch rejections show a localized error", async ({ page }) => {
  await page.goto(
    "/tests/fixtures/production-settings-control-widths.html?rejectMetaOpen=1",
  );

  await page
    .getByTestId("production-transcription")
    .getByRole("button", { name: "Open Meta AI", exact: true })
    .click();

  await expect(page.getByText("Transcription Failed")).toBeVisible();
  await expect(
    page.getByText(
      "Meta AI could not be opened. Open it manually from Applications.",
    ),
  ).toBeVisible();
});
