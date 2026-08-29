import { expect, test, type Locator, type Page } from "@playwright/test";

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

test.describe("settings control widths", () => {
  test.beforeEach(async ({ page }) => {
    await page.setViewportSize({ width: 960, height: 1400 });
    await page.goto("/tests/fixtures/settings-control-widths.html");
  });

  test("keeps every compound primary control on the full rail", async ({
    page,
  }) => {
    const directTrigger = page
      .getByRole("button", { name: "First option" })
      .first();
    const directSlot = page
      .getByTestId("direct-dropdown")
      .locator('xpath=ancestor::*[@data-slot="setting-control"][1]');
    const [directTriggerBox, directSlotBox] = await Promise.all([
      boxOf(directTrigger),
      boxOf(directSlot),
    ]);

    expectWidth(directTriggerBox.width, RAIL_WIDTH);
    expectWidth(directSlotBox.width, RAIL_WIDTH);

    const groups = page.locator('[data-slot="control-group"]');
    await expect(groups).toHaveCount(4);

    for (let index = 0; index < (await groups.count()); index += 1) {
      const group = groups.nth(index);
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
        directSlotBox.x + directSlotBox.width,
        1,
      );
      expect(groupBox.x).toBeGreaterThanOrEqual(rowBox.x);
      expect(groupBox.x + groupBox.width).toBeLessThanOrEqual(
        rowBox.x + rowBox.width + WIDTH_TOLERANCE,
      );
    }

    await directTrigger.click();
    const panel = page.locator("[data-floating-panel-root]");
    await expect(panel).toBeVisible();
    expectWidth((await boxOf(panel)).width, RAIL_WIDTH);

    const toggle = page
      .getByRole("checkbox", { name: "Recording indicator" })
      .locator("..");
    const toggleBox = await boxOf(toggle);
    expect(toggleBox.width).toBeLessThan(RAIL_WIDTH);
    expect(toggleBox.x + toggleBox.width).toBeCloseTo(
      directSlotBox.x + directSlotBox.width,
      1,
    );

    const historySlot = page
      .getByTestId("history-limit")
      .locator('xpath=ancestor::*[@data-slot="setting-control"][1]');
    const geminiSlot = page
      .getByTestId("gemini-status")
      .locator('xpath=ancestor::*[@data-slot="setting-control"][1]');
    expectWidth((await boxOf(historySlot)).width, RAIL_WIDTH);
    expectWidth((await boxOf(geminiSlot)).width, RAIL_WIDTH);

    const chipLane = page.getByTestId("custom-word-chips");
    const chipMeasurements = await chipLane.evaluate((element) => ({
      clientWidth: element.clientWidth,
      scrollWidth: element.scrollWidth,
      width: element.getBoundingClientRect().width,
    }));
    expectWidth(chipMeasurements.width, RAIL_WIDTH);
    expect(chipMeasurements.scrollWidth).toBeLessThanOrEqual(
      chipMeasurements.clientWidth,
    );

    const pathDisplay = page.getByTestId("path-display");
    const pathSurface = pathDisplay.locator('[data-slot="path-surface"]');
    const openButton = pathDisplay.getByRole("button", { name: "Open" });
    const [pathDisplayBox, pathSurfaceBox, openButtonBox] = await Promise.all([
      boxOf(pathDisplay),
      boxOf(pathSurface),
      boxOf(openButton),
    ]);
    expectWidth(pathSurfaceBox.width, pathDisplayBox.width);
    expect(openButtonBox.y).toBeGreaterThanOrEqual(
      pathSurfaceBox.y + pathSurfaceBox.height,
    );
    expect(openButtonBox.x + openButtonBox.width).toBeCloseTo(
      pathSurfaceBox.x + pathSurfaceBox.width,
      1,
    );

    await expectNoHorizontalOverflow(page);
  });

  test("remains contained at Murmur's minimum content width", async ({
    page,
  }) => {
    await page.setViewportSize({ width: 520, height: 1400 });
    await page.reload();

    const section = page.getByTestId("settings-control-widths");
    const sectionBox = await boxOf(section);
    const clientWidth = await page.evaluate(
      () => document.documentElement.clientWidth,
    );
    expectWidth(sectionBox.width, clientWidth - 30);

    const groups = page.locator('[data-slot="control-group"]');
    for (let index = 0; index < (await groups.count()); index += 1) {
      const group = groups.nth(index);
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

    await expectNoHorizontalOverflow(page);
  });
});
