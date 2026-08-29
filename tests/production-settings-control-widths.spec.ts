import { expect, test, type Locator, type Page } from "@playwright/test";

const APP_DATA_PATH =
  "/Users/example/Library/Application Support/com.dailyxplorer.murmur";
const LOG_DIRECTORY_PATH =
  "/Users/example/Library/Logs/com.dailyxplorer.murmur";
const LONG_CUSTOM_WORD = "W".repeat(50);
const RAIL_WIDTH = 260;
const WIDTH_TOLERANCE = 1;

const expectWidth = (width: number, expected: number) => {
  expect(Math.abs(width - expected)).toBeLessThanOrEqual(WIDTH_TOLERANCE);
};

const boxOf = async (locator: Locator) => {
  const box = await locator.boundingBox();
  expect(box).not.toBeNull();
  return box as NonNullable<typeof box>;
};

const expectNoHorizontalOverflow = async (page: Page) => {
  const sizes = await page.evaluate(() => ({
    clientWidth: document.documentElement.clientWidth,
    scrollWidth: document.documentElement.scrollWidth,
  }));
  expect(sizes.scrollWidth).toBeLessThanOrEqual(sizes.clientWidth);
};

const compoundGroup = (component: Locator) =>
  component.locator('[data-slot="control-group"]');

const expectFullWidthPath = async (component: Locator, path: string) => {
  const surface = component.locator('[data-slot="path-surface"]');
  const root = surface.locator("..");
  const openButton = component.getByRole("button", { name: "Open" });
  await expect(component.getByText(path, { exact: true })).toBeVisible();

  const [surfaceBox, rootBox, openButtonBox] = await Promise.all([
    boxOf(surface),
    boxOf(root),
    boxOf(openButton),
  ]);
  expectWidth(surfaceBox.width, rootBox.width);
  expect(openButtonBox.y).toBeGreaterThanOrEqual(
    surfaceBox.y + surfaceBox.height,
  );
  expect(openButtonBox.x + openButtonBox.width).toBeCloseTo(
    surfaceBox.x + surfaceBox.width,
    1,
  );
};

test.describe("production settings control widths", () => {
  test.beforeEach(async ({ page }) => {
    await page.setViewportSize({ width: 960, height: 2400 });
    await page.goto("/tests/fixtures/production-settings-control-widths.html");
  });

  test("keeps real compound primary controls on the full rail", async ({
    page,
  }) => {
    const microphoneComponent = page.getByTestId("production-microphone");
    const languageComponent = page.getByTestId("production-language");
    const transcribeComponent = page.getByTestId(
      "production-transcribe-shortcut",
    );
    const cancelComponent = page.getByTestId("production-cancel-shortcut");
    const customWordsComponent = page.getByTestId("production-custom-words");
    const providerSlot = page
      .getByTestId("production-transcription")
      .locator('[data-slot="setting-control"]')
      .first();
    const provider = providerSlot.getByRole("button").first();

    await expect(
      page
        .getByTestId("production-transcription")
        .getByText("Connected", { exact: true })
        .last(),
    ).toBeVisible();

    const providerBox = await boxOf(provider);
    const providerSlotBox = await boxOf(providerSlot);
    expectWidth(providerBox.width, RAIL_WIDTH);
    expectWidth(providerSlotBox.width, RAIL_WIDTH);

    const components = [
      microphoneComponent,
      languageComponent,
      transcribeComponent,
      cancelComponent,
      customWordsComponent,
    ];

    for (const component of components) {
      const group = compoundGroup(component);
      const primary = group.locator('[data-slot="control-primary"]');
      const action = group.locator('[data-slot="control-action"]');
      const slot = group.locator(
        'xpath=ancestor::*[@data-slot="setting-control"][1]',
      );
      const row = slot.locator("..");
      const [groupBox, primaryBox, actionBox, rowBox] = await Promise.all([
        boxOf(group),
        boxOf(primary),
        boxOf(action),
        boxOf(row),
      ]);

      expectWidth(primaryBox.width, RAIL_WIDTH);
      expect(actionBox.width).toBeLessThan(RAIL_WIDTH);
      expect(groupBox.width).toBeGreaterThan(RAIL_WIDTH);
      expect(groupBox.x + groupBox.width).toBeCloseTo(
        providerSlotBox.x + providerSlotBox.width,
        1,
      );
      expect(groupBox.x).toBeGreaterThanOrEqual(rowBox.x);
      expect(groupBox.x + groupBox.width).toBeLessThanOrEqual(
        rowBox.x + rowBox.width + WIDTH_TOLERANCE,
      );
    }

    const microphone = microphoneComponent.getByRole("button", {
      name: "Default",
    });
    const language = languageComponent.getByRole("button", {
      name: "Auto Detect",
    });
    const transcribe = transcribeComponent.getByText("Alt + Space", {
      exact: true,
    });
    const cancel = cancelComponent.getByText("Escape", { exact: true });
    const customWordsInput =
      customWordsComponent.getByPlaceholder("Add a word");

    for (const control of [
      microphone,
      language,
      transcribe,
      cancel,
      customWordsInput,
    ]) {
      expectWidth((await boxOf(control)).width, RAIL_WIDTH);
    }

    const pushComponent = page.getByTestId("production-push-to-talk");
    const pushToggle = pushComponent
      .getByRole("checkbox", { name: "Push To Talk" })
      .locator("..");
    const pushSlot = pushToggle.locator(
      'xpath=ancestor::*[@data-slot="setting-control"][1]',
    );
    const [pushToggleBox, pushSlotBox] = await Promise.all([
      boxOf(pushToggle),
      boxOf(pushSlot),
    ]);
    expect(pushToggleBox.width).toBeLessThan(RAIL_WIDTH);
    expectWidth(pushSlotBox.width, RAIL_WIDTH);
    expect(pushToggleBox.x + pushToggleBox.width).toBeCloseTo(
      providerSlotBox.x + providerSlotBox.width,
      1,
    );

    const historySlot = page
      .getByTestId("production-history-limit")
      .getByText("entries", { exact: true })
      .locator('xpath=ancestor::*[@data-slot="setting-control"][1]');
    expectWidth((await boxOf(historySlot)).width, RAIL_WIDTH);

    const longChip = customWordsComponent.getByTitle(LONG_CUSTOM_WORD);
    const chipLane = longChip.locator("..");
    const chipMeasurements = await chipLane.evaluate((element) => ({
      clientWidth: element.clientWidth,
      scrollWidth: element.scrollWidth,
      width: element.getBoundingClientRect().width,
    }));
    expectWidth(chipMeasurements.width, RAIL_WIDTH);
    expect(chipMeasurements.scrollWidth).toBeLessThanOrEqual(
      chipMeasurements.clientWidth,
    );

    await expectFullWidthPath(
      page.getByTestId("production-app-data"),
      APP_DATA_PATH,
    );
    await expectFullWidthPath(
      page.getByTestId("production-log-directory"),
      LOG_DIRECTORY_PATH,
    );
    await expectNoHorizontalOverflow(page);
  });

  test("contains production groups at Murmur's minimum content width", async ({
    page,
  }) => {
    await page.setViewportSize({ width: 520, height: 2400 });
    await page.reload();

    const components = [
      page.getByTestId("production-microphone"),
      page.getByTestId("production-language"),
      page.getByTestId("production-transcribe-shortcut"),
      page.getByTestId("production-cancel-shortcut"),
      page.getByTestId("production-custom-words"),
    ];

    for (const component of components) {
      const group = compoundGroup(component);
      const primary = group.locator('[data-slot="control-primary"]');
      const slot = group.locator(
        'xpath=ancestor::*[@data-slot="setting-control"][1]',
      );
      const row = slot.locator("..");
      const [groupBox, primaryBox, rowBox] = await Promise.all([
        boxOf(group),
        boxOf(primary),
        boxOf(row),
      ]);

      expectWidth(primaryBox.width, RAIL_WIDTH);
      expect(groupBox.x).toBeGreaterThanOrEqual(rowBox.x);
      expect(groupBox.x + groupBox.width).toBeLessThanOrEqual(
        rowBox.x + rowBox.width + WIDTH_TOLERANCE,
      );
    }

    await expectFullWidthPath(
      page.getByTestId("production-app-data"),
      APP_DATA_PATH,
    );
    await expectFullWidthPath(
      page.getByTestId("production-log-directory"),
      LOG_DIRECTORY_PATH,
    );
    await expectNoHorizontalOverflow(page);
  });
});
