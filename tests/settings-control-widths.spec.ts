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
      chipLaneOverflow,
      historyLimitLabelBox,
      historyLimitBox,
      longChipBox,
      shortcutSurfaceBox,
      shortcutResetBox,
      shortcutTextAlign,
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
      page.getByTestId("custom-word-chips").evaluate((element) => ({
        clientWidth: element.clientWidth,
        scrollWidth: element.scrollWidth,
      })),
      page.getByTestId("history-limit").getByText("entries").boundingBox(),
      page.getByTestId("history-limit").boundingBox(),
      page.getByTestId("long-custom-word").boundingBox(),
      page
        .getByTestId("shortcut-control")
        .getByTestId("shortcut-surface")
        .boundingBox(),
      page
        .getByTestId("shortcut-control")
        .getByRole("button", { name: "Reset shortcut" })
        .boundingBox(),
      page
        .getByTestId("shortcut-control")
        .getByTestId("shortcut-surface")
        .evaluate((element) => getComputedStyle(element).textAlign),
      page.getByTestId("stacked-control").boundingBox(),
    ]);

    for (const box of [resetBox, addBox, toggleBox, chipBox]) {
      expect(box).not.toBeNull();
      expect(box?.width ?? RAIL_WIDTH).toBeLessThan(RAIL_WIDTH);
    }
    expect(chipLaneBox).not.toBeNull();
    expect(chipLaneBox?.width ?? 0).toBeGreaterThan(RAIL_WIDTH);
    expect(historyLimitBox).not.toBeNull();
    expect(
      (historyLimitBox?.x ?? 0) + (historyLimitBox?.width ?? 0),
    ).toBeCloseTo(slotMeasurements[4].right, 1);
    expect(historyLimitLabelBox).not.toBeNull();
    expect(
      (historyLimitLabelBox?.x ?? 0) + (historyLimitLabelBox?.width ?? 0),
    ).toBeCloseTo(slotMeasurements[4].right, 1);
    expect(chipLaneOverflow.scrollWidth).toBeLessThanOrEqual(
      chipLaneOverflow.clientWidth,
    );
    expect(longChipBox).not.toBeNull();
    expect(longChipBox?.width ?? 0).toBeLessThanOrEqual(
      chipLaneBox?.width ?? RAIL_WIDTH,
    );
    expect(chipBox?.x ?? 0).toBeCloseTo(chipLaneBox?.x ?? 0, 1);

    const longTitle = page.getByRole("heading", {
      name: "Supprimer automatiquement tous les mots de remplissage détectés pendant la transcription",
    });
    const infoTrigger = page.getByRole("button", {
      name: "A long French label must stay on one line",
    });
    const [titleMetrics, infoBox] = await Promise.all([
      longTitle.evaluate((element) => {
        const rect = element.getBoundingClientRect();
        const styles = getComputedStyle(element);
        return {
          clientWidth: element.clientWidth,
          height: rect.height,
          lineHeight: Number.parseFloat(styles.lineHeight),
          right: rect.right,
          scrollWidth: element.scrollWidth,
        };
      }),
      infoTrigger.boundingBox(),
    ]);
    expect(titleMetrics.scrollWidth).toBeGreaterThan(titleMetrics.clientWidth);
    expect(titleMetrics.height).toBeLessThanOrEqual(
      titleMetrics.lineHeight + 1,
    );
    expect(infoBox).not.toBeNull();
    const titleToTriggerGap = (infoBox?.x ?? 0) - titleMetrics.right;
    expect(titleToTriggerGap).toBeGreaterThanOrEqual(0);
    expect(titleToTriggerGap).toBeLessThan(8);
    const titleOwnsItsRightEdge = await longTitle.evaluate((element) => {
      const rect = element.getBoundingClientRect();
      const hit = document.elementFromPoint(
        rect.right - 4,
        rect.top + rect.height / 2,
      );
      return Boolean(hit && (hit === element || element.contains(hit)));
    });
    expect(titleOwnsItsRightEdge).toBe(true);
    const [infoHitAreaRight, customWordsInputBox] = await Promise.all([
      infoTrigger.evaluate((element) => {
        const rect = element.getBoundingClientRect();
        const styles = getComputedStyle(element, "::after");
        return (
          rect.left +
          Number.parseFloat(styles.left) +
          Number.parseFloat(styles.width)
        );
      }),
      page.getByPlaceholder("Add a custom word").boundingBox(),
    ]);
    expect(customWordsInputBox).not.toBeNull();
    expect(infoHitAreaRight).toBeLessThanOrEqual(customWordsInputBox?.x ?? 0);

    const stackedInfoTrigger = page.getByRole("button", {
      name: "This content intentionally remains full width",
    });
    const stackedContainer = page
      .getByTestId("stacked-control")
      .locator("..")
      .locator("..");
    const [stackedHitArea, stackedContainerBox] = await Promise.all([
      stackedInfoTrigger.evaluate((element) => {
        const rect = element.getBoundingClientRect();
        const styles = getComputedStyle(element, "::after");
        const height = Number.parseFloat(styles.height);
        const center = rect.top + rect.height / 2;
        return {
          bottom: center + height / 2,
          top: center - height / 2,
        };
      }),
      stackedContainer.boundingBox(),
    ]);
    expect(stackedContainerBox).not.toBeNull();
    expect(stackedHitArea.top).toBeGreaterThanOrEqual(
      stackedContainerBox?.y ?? 0,
    );
    expect(stackedHitArea.bottom).toBeLessThanOrEqual(stackedBox?.y ?? 0);

    expect(shortcutSurfaceBox).not.toBeNull();
    expect(shortcutResetBox).not.toBeNull();
    expect(shortcutTextAlign).toBe("start");
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

  test("keeps the enlarged info target clear in right-to-left layouts", async ({
    page,
  }) => {
    await page
      .getByTestId("settings-control-widths")
      .evaluate((element) => element.setAttribute("dir", "rtl"));

    const longTitle = page.getByRole("heading", {
      name: "Supprimer automatiquement tous les mots de remplissage détectés pendant la transcription",
    });
    const infoTrigger = page.getByRole("button", {
      name: "A long French label must stay on one line",
    });
    const customWordsInput = page.getByPlaceholder("Add a custom word");
    const [titleBox, infoBox, customWordsInputBox] = await Promise.all([
      longTitle.boundingBox(),
      infoTrigger.boundingBox(),
      customWordsInput.boundingBox(),
    ]);

    expect(titleBox).not.toBeNull();
    expect(infoBox).not.toBeNull();
    expect(customWordsInputBox).not.toBeNull();
    const titleToTriggerGap =
      (titleBox?.x ?? 0) - ((infoBox?.x ?? 0) + (infoBox?.width ?? 0));
    expect(titleToTriggerGap).toBeGreaterThanOrEqual(0);
    expect(titleToTriggerGap).toBeLessThan(8);

    const titleOwnsItsLeftEdge = await longTitle.evaluate((element) => {
      const rect = element.getBoundingClientRect();
      const hit = document.elementFromPoint(
        rect.left + 4,
        rect.top + rect.height / 2,
      );
      return Boolean(hit && (hit === element || element.contains(hit)));
    });
    expect(titleOwnsItsLeftEdge).toBe(true);

    const infoHitAreaLeft = await infoTrigger.evaluate((element) => {
      const rect = element.getBoundingClientRect();
      const styles = getComputedStyle(element, "::after");
      return (
        rect.right -
        Number.parseFloat(styles.right) -
        Number.parseFloat(styles.width)
      );
    });
    expect(infoHitAreaLeft).toBeGreaterThanOrEqual(
      (customWordsInputBox?.x ?? 0) + (customWordsInputBox?.width ?? 0),
    );
  });
});
