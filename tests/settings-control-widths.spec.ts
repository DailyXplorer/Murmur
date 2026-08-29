import { expect, test } from "@playwright/test";

const RAIL_WIDTH = 260;
const WIDTH_TOLERANCE = 1;

const expectWidth = (width: number, expected: number) => {
  expect(Math.abs(width - expected)).toBeLessThanOrEqual(WIDTH_TOLERANCE);
};

test.describe("settings control widths", () => {
  test.beforeEach(async ({ page }) => {
    await page.setViewportSize({ width: 960, height: 1200 });
    await page.goto("/tests/fixtures/settings-control-widths.html");
  });

  test("reserves one aligned control rail without stretching compact actions", async ({
    page,
  }) => {
    const horizontalControlMarkers = [
      page.getByTestId("direct-dropdown"),
      page.getByTestId("dropdown-reset"),
      page.getByTestId("language-picker"),
      page.getByTestId("shortcut-control"),
      page.getByTestId("history-limit"),
      page.getByTestId("custom-words"),
      page.getByTestId("gemini-status"),
    ];
    const slots = horizontalControlMarkers.map((marker) =>
      marker.locator(".."),
    );
    const toggleSlot = page
      .getByRole("checkbox", { name: "Recording indicator" })
      .locator("..")
      .locator("..");
    slots.push(toggleSlot);

    const slotMeasurements = await Promise.all(
      slots.map(async (slot) =>
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

    for (const measurement of slotMeasurements) {
      expectWidth(measurement.width, RAIL_WIDTH);
      expect(measurement.scrollWidth).toBeLessThanOrEqual(
        measurement.clientWidth,
      );
      expect(measurement.right).toBeCloseTo(slotMeasurements[0].right, 1);
    }

    const directTrigger = page
      .getByRole("button", { name: "First option" })
      .first();
    const directTriggerBox = await directTrigger.boundingBox();
    expect(directTriggerBox).not.toBeNull();
    expectWidth(directTriggerBox?.width ?? 0, RAIL_WIDTH);

    await directTrigger.click();
    const panel = page.locator("[data-floating-panel-root]");
    await expect(panel).toBeVisible();
    const panelBox = await panel.boundingBox();
    expect(panelBox).not.toBeNull();
    expectWidth(panelBox?.width ?? 0, RAIL_WIDTH);

    const [
      resetBox,
      addBox,
      toggleBox,
      chipBox,
      chipLaneBox,
      shortcutSurfaceBox,
      shortcutResetBox,
      stackedBox,
    ] = await Promise.all([
      page
        .getByTestId("dropdown-reset")
        .getByRole("button", { name: "Reset dropdown" })
        .boundingBox(),
      page
        .getByTestId("custom-words")
        .getByRole("button", { name: "Add" })
        .boundingBox(),
      page
        .getByRole("checkbox", { name: "Recording indicator" })
        .locator("..")
        .boundingBox(),
      page
        .getByTestId("custom-word-chips")
        .getByRole("button", { name: "Acme" })
        .boundingBox(),
      page.getByTestId("custom-word-chips").boundingBox(),
      page
        .getByTestId("shortcut-control")
        .getByRole("button", { name: "Command Shift Option K" })
        .boundingBox(),
      page
        .getByTestId("shortcut-control")
        .getByRole("button", { name: "Reset shortcut" })
        .boundingBox(),
      page.getByTestId("stacked-control").boundingBox(),
    ]);

    for (const box of [resetBox, addBox, toggleBox, chipBox]) {
      expect(box).not.toBeNull();
      expect(box?.width ?? RAIL_WIDTH).toBeLessThan(RAIL_WIDTH);
    }
    expect(chipLaneBox).not.toBeNull();
    expectWidth(chipLaneBox?.width ?? 0, RAIL_WIDTH);
    expect(shortcutSurfaceBox).not.toBeNull();
    expect(shortcutResetBox).not.toBeNull();
    expect(
      (shortcutResetBox?.x ?? 0) +
        (shortcutResetBox?.width ?? 0) -
        (shortcutSurfaceBox?.x ?? 0),
    ).toBeCloseTo(RAIL_WIDTH, 1);
    expect((toggleBox?.x ?? 0) + (toggleBox?.width ?? 0)).toBeCloseTo(
      slotMeasurements[slotMeasurements.length - 1].right,
      1,
    );
    expect(stackedBox).not.toBeNull();
    expect(stackedBox?.width ?? 0).toBeGreaterThan(RAIL_WIDTH);
  });
});
