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
    expect((panelBox?.y ?? 0) + (panelBox?.height ?? 0)).toBeGreaterThan(
      footerBox?.y ?? 400,
    );

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
    await expect(page.getByRole("listbox")).toHaveCount(0);

    await page.getByRole("button", { name: "Option 3" }).click();
    await page.keyboard.press("Escape");
    await expect(page.getByRole("listbox")).toHaveCount(0);
    await expect(page.getByRole("button", { name: "Option 3" })).toBeFocused();

    await page.getByRole("button", { name: "Option 3" }).click();
    await page.getByTestId("footer").click();
    await expect(page.getByRole("listbox")).toHaveCount(0);
  });
});
