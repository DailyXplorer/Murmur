import { expect, test } from "@playwright/test";

test.describe("floating settings panels", () => {
  test.beforeEach(async ({ page }) => {
    await page.setViewportSize({ width: 800, height: 400 });
    await page.goto("/tests/fixtures/floating-panel.html");
  });

  test("escapes clipping containers and stays inside the viewport", async ({
    page,
  }) => {
    await page.getByRole("button", { name: "Select an option..." }).click();

    const menu = page.getByRole("listbox");
    const panel = page.locator("[data-placement]");
    const trigger = page.getByRole("button", { name: "Select an option..." });
    await expect(menu).toBeVisible();
    await expect(menu).toHaveAccessibleName("Select an option...");
    await expect(
      page.getByRole("option", { name: "Option 1", exact: true }),
    ).toBeFocused();
    await page.keyboard.press("ArrowDown");
    await expect(page.getByRole("option", { name: "Option 2" })).toBeFocused();
    await page.keyboard.press("End");
    await expect(page.getByRole("option", { name: "Option 12" })).toBeFocused();
    await page.keyboard.press("Home");
    await expect(
      page.getByRole("option", { name: "Option 1", exact: true }),
    ).toBeFocused();
    await expect(panel).toHaveAttribute("data-placement", "top");

    const [panelBox, triggerBox] = await Promise.all([
      panel.boundingBox(),
      trigger.boundingBox(),
    ]);
    expect(panelBox).not.toBeNull();
    expect(triggerBox).not.toBeNull();
    expect(panelBox?.y).toBeGreaterThanOrEqual(8);
    expect((panelBox?.y ?? 0) + (panelBox?.height ?? 0)).toBeLessThanOrEqual(
      400,
    );
    expect(panelBox?.height).toBeGreaterThan(100);
    expect((panelBox?.y ?? 0) + (panelBox?.height ?? 0)).toBeLessThanOrEqual(
      triggerBox?.y ?? 0,
    );

    await page.setViewportSize({ width: 800, height: 700 });
    await expect(panel).toHaveAttribute("data-placement", "bottom");
    const [resizedPanelBox, resizedTriggerBox] = await Promise.all([
      panel.boundingBox(),
      trigger.boundingBox(),
    ]);
    expect(resizedPanelBox?.y).toBeGreaterThanOrEqual(
      (resizedTriggerBox?.y ?? 0) + (resizedTriggerBox?.height ?? 0),
    );

    await trigger.evaluate((element) => {
      element.parentElement?.style.setProperty(
        "transform",
        "translateY(-500px)",
      );
      window.dispatchEvent(new Event("scroll"));
    });
    await expect(page.getByRole("listbox")).toHaveCount(0);
  });

  test("renders above a later high-z-index footer", async ({ page }) => {
    await page.goto("/tests/fixtures/floating-panel.html?options=3");
    await page.getByRole("button", { name: "Select an option..." }).click();

    const panel = page.locator("[data-placement]");
    const footer = page.getByTestId("footer");
    await expect(panel).toHaveAttribute("data-placement", "bottom");
    const [panelBox, footerBox] = await Promise.all([
      panel.boundingBox(),
      footer.boundingBox(),
    ]);
    expect(panelBox).not.toBeNull();
    expect(footerBox).not.toBeNull();
    if (!panelBox || !footerBox) throw new Error("Expected visible elements");

    const overlapLeft = Math.max(panelBox.x, footerBox.x);
    const overlapRight = Math.min(
      panelBox.x + panelBox.width,
      footerBox.x + footerBox.width,
    );
    const overlapTop = Math.max(panelBox.y, footerBox.y);
    const overlapBottom = Math.min(
      panelBox.y + panelBox.height,
      footerBox.y + footerBox.height,
    );
    expect(overlapRight).toBeGreaterThan(overlapLeft);
    expect(overlapBottom).toBeGreaterThan(overlapTop);

    const panelPaintsOnTop = await panel.evaluate(
      (element, point) => {
        const paintedElement = document.elementFromPoint(point.x, point.y);
        return (
          paintedElement !== null &&
          (paintedElement === element || element.contains(paintedElement))
        );
      },
      {
        x: (overlapLeft + overlapRight) / 2,
        y: (overlapTop + overlapBottom) / 2,
      },
    );
    expect(panelPaintsOnTop).toBe(true);

    await page.getByRole("option", { name: "Option 2", exact: true }).click();
    await expect(page.getByRole("button", { name: "Option 2" })).toBeVisible();
  });

  test("supports selection, Escape, and outside-click dismissal", async ({
    page,
  }) => {
    const trigger = page.getByRole("button", { name: "Select an option..." });
    await trigger.click();
    await page.getByRole("option", { name: "Option 3", exact: true }).click();
    await expect(page.getByRole("button", { name: "Option 3" })).toBeVisible();
    await expect(page.getByRole("button", { name: "Option 3" })).toBeFocused();
    await expect(page.getByRole("listbox")).toHaveCount(0);

    await page.getByRole("button", { name: "Option 3" }).click();
    await page.keyboard.press("Escape");
    await expect(page.getByRole("listbox")).toHaveCount(0);
    await expect(page.getByRole("button", { name: "Option 3" })).toBeFocused();

    await page.getByRole("button", { name: "Option 3" }).click();
    const outsideControl = page.getByTestId("outside-control");
    await outsideControl.click();
    await expect(page.getByRole("listbox")).toHaveCount(0);
    await expect(outsideControl).toBeFocused();
  });

  test("preserves editing keys in embedded search inputs", async ({ page }) => {
    await page.goto("/tests/fixtures/floating-panel.html?search=1");
    await page.getByRole("button", { name: "Open searchable panel" }).click();

    const searchInput = page.getByRole("textbox", { name: "Search options" });
    await searchInput.fill("search text");
    await searchInput.press("Home");
    await expect(searchInput).toBeFocused();
    await expect
      .poll(() => searchInput.evaluate((input) => input.selectionStart))
      .toBe(0);

    await searchInput.press("End");
    await expect(searchInput).toBeFocused();
    await expect
      .poll(() => searchInput.evaluate((input) => input.selectionStart))
      .toBe("search text".length);

    await searchInput.press("ArrowUp");
    await expect(searchInput).toBeFocused();
    await searchInput.press("ArrowDown");
    await expect(searchInput).toBeFocused();
  });

  test("lets panel content handle Escape before dismissing", async ({
    page,
  }) => {
    await page.goto("/tests/fixtures/floating-panel.html?search=1");
    await page.getByRole("button", { name: "Open searchable panel" }).click();

    const searchInput = page.getByRole("textbox", { name: "Search options" });
    await searchInput.press("Escape");
    await expect(page.getByText("Last input key: Escape")).toBeVisible();
    await expect(page.locator("[data-floating-panel-root]")).toHaveCount(1);
  });

  test("dismisses only the topmost panel with Escape", async ({ page }) => {
    await page.goto("/tests/fixtures/floating-panel.html?multiple=1");
    await expect(page.getByText("First panel", { exact: true })).toBeVisible();
    await expect(page.getByText("Second panel", { exact: true })).toBeVisible();

    await page.keyboard.press("Escape");
    await expect(page.getByText("Second panel", { exact: true })).toHaveCount(
      0,
    );
    await expect(page.getByText("First panel", { exact: true })).toBeVisible();

    await page.keyboard.press("Escape");
    await expect(page.getByText("First panel", { exact: true })).toHaveCount(0);
  });

  test("dismisses when keyboard focus leaves the panel", async ({ page }) => {
    await page.goto("/tests/fixtures/floating-panel.html?options=3");
    await page.getByRole("button", { name: "Select an option..." }).click();
    await expect(
      page.getByRole("option", { name: "Option 1", exact: true }),
    ).toBeFocused();

    await page.keyboard.press("Shift+Tab");
    await expect(page.getByTestId("outside-control")).toBeFocused();
    await expect(page.getByRole("listbox")).toHaveCount(0);
  });

  test("keeps dialogs above unrelated panels and nested panels above dialogs", async ({
    page,
  }) => {
    await page.goto("/tests/fixtures/floating-panel.html?dialog-layers=1");

    const outsidePanel = page
      .locator("[data-floating-panel-root]")
      .filter({ has: page.getByTestId("outside-dialog-panel") });
    const nestedPanel = page
      .locator("[data-floating-panel-root]")
      .filter({ has: page.getByTestId("nested-dialog-panel") });
    const dialog = page.getByRole("dialog", { name: "Layering dialog" });
    await expect(outsidePanel).toBeVisible();
    await expect(nestedPanel).toBeVisible();
    await expect(dialog).toBeVisible();

    const [outsideZIndex, dialogZIndex, nestedZIndex] = await Promise.all([
      outsidePanel.evaluate((element) =>
        Number(getComputedStyle(element).zIndex),
      ),
      dialog.evaluate((element) =>
        Number(getComputedStyle(element.parentElement as HTMLElement).zIndex),
      ),
      nestedPanel.evaluate((element) =>
        Number(getComputedStyle(element).zIndex),
      ),
    ]);
    expect(outsideZIndex).toBeLessThan(dialogZIndex);
    expect(nestedZIndex).toBeGreaterThan(dialogZIndex);

    const outsidePanelPaintsOnTop = await outsidePanel.evaluate((element) => {
      const bounds = element.getBoundingClientRect();
      const paintedElement = document.elementFromPoint(
        bounds.left + bounds.width / 2,
        bounds.top + bounds.height / 2,
      );
      return paintedElement === element || element.contains(paintedElement);
    });
    const nestedPanelPaintsOnTop = await nestedPanel.evaluate((element) => {
      const bounds = element.getBoundingClientRect();
      const paintedElement = document.elementFromPoint(
        bounds.left + bounds.width / 2,
        bounds.top + bounds.height / 2,
      );
      return paintedElement === element || element.contains(paintedElement);
    });
    expect(outsidePanelPaintsOnTop).toBe(false);
    expect(nestedPanelPaintsOnTop).toBe(true);

    await page.keyboard.press("Escape");
    await expect(nestedPanel).toHaveCount(0);
    await expect(dialog).toBeVisible();

    await page.keyboard.press("Escape");
    await expect(outsidePanel).toHaveCount(0);
    await expect(dialog).toBeVisible();

    await page.keyboard.press("Escape");
    await expect(dialog).toHaveCount(0);
  });
});
