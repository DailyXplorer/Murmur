import { expect, test, type Page } from "@playwright/test";

const CONTROL_HEIGHT = 37.5;
const BUTTON_MIN_WIDTH = 90;
const RAIL_WIDTH = 260;
const TOLERANCE = 1;

const SECTIONS = [
  "General",
  "Transcription",
  "History",
  "Insights",
  "Advanced",
  "Debug",
  "About",
];

const sizeViolations = (page: Page) =>
  page.evaluate(
    ({ controlHeight, buttonMinWidth, railWidth, tolerance }) => {
      const visible = (element: Element) => element.getClientRects().length > 0;
      const describe = (element: Element) =>
        `${element.tagName.toLowerCase()} "${(
          element.getAttribute("aria-label") ??
          element.getAttribute("placeholder") ??
          (element as HTMLElement).innerText ??
          ""
        )
          .replace(/\s+/g, " ")
          .slice(0, 40)}"`;
      const violations: string[] = [];

      const fieldLike = document.querySelectorAll(
        'input:not([type="checkbox"]):not([type="radio"]):not([type="range"]), [aria-haspopup="listbox"]',
      );
      for (const element of Array.from(fieldLike).filter(visible)) {
        if (!element.hasAttribute("data-control")) {
          violations.push(`${describe(element)} bypasses the shared controls`);
        }
      }

      const controls = Array.from(
        document.querySelectorAll("[data-control]"),
      ).filter(visible);
      for (const element of controls) {
        const box = element.getBoundingClientRect();
        const kind = element.getAttribute("data-control");
        if (Math.abs(box.height - controlHeight) > tolerance) {
          violations.push(`${describe(element)} is ${box.height}px tall`);
        }
        if (kind === "icon" && Math.abs(box.width - box.height) > tolerance) {
          violations.push(`${describe(element)} icon button is not square`);
        }
        if (kind === "text") {
          if (box.width < buttonMinWidth - tolerance) {
            violations.push(`${describe(element)} is ${box.width}px wide`);
          }
          if (element.scrollWidth > element.clientWidth) {
            violations.push(`${describe(element)} label overflows`);
          }
        }
      }

      const rails = new Set(
        controls
          .filter((element) => element.getAttribute("data-control") === "field")
          .map((element) =>
            element.closest('[class*="settings-control-rail-width"]'),
          )
          .filter((rail): rail is Element => rail !== null),
      );
      for (const rail of rails) {
        const railBox = rail.getBoundingClientRect();
        const railControls = controls.filter((element) =>
          rail.contains(element),
        );
        const left = Math.min(
          ...railControls.map(
            (element) => element.getBoundingClientRect().left,
          ),
        );
        if (Math.abs(left - railBox.left) > tolerance) {
          violations.push(
            `${describe(railControls[0])} starts ${left - railBox.left}px into the rail`,
          );
        }
        if (Math.abs(railBox.width - railWidth) > tolerance) {
          violations.push(
            `${describe(railControls[0])} rail is ${railBox.width}px`,
          );
        }
      }

      return { controls: controls.length, violations };
    },
    {
      buttonMinWidth: BUTTON_MIN_WIDTH,
      controlHeight: CONTROL_HEIGHT,
      railWidth: RAIL_WIDTH,
      tolerance: TOLERANCE,
    },
  );

for (const language of ["en", "fr"]) {
  test.describe(`settings control sizes (${language})`, () => {
    test.beforeEach(async ({ page }) => {
      await page.setViewportSize({ width: 960, height: 2400 });
    });

    for (const [index, section] of SECTIONS.entries()) {
      test(section, async ({ page }) => {
        await page.goto(`/tests/fixtures/settings-pages.html?lang=${language}`);
        const sectionButton = page
          .getByRole("navigation")
          .getByRole("button")
          .nth(index);
        await sectionButton.click();
        await expect(sectionButton).toHaveAttribute("aria-current", "page");
        await page.waitForTimeout(300);

        const { violations } = await sizeViolations(page);
        expect(violations).toEqual([]);
      });
    }

    test("language picker search field", async ({ page }) => {
      await page.goto(`/tests/fixtures/settings-pages.html?lang=${language}`);
      await page.locator('[aria-haspopup="listbox"]').nth(2).click();
      await expect(
        page.locator("[data-floating-panel-root] input"),
      ).toBeVisible();

      const { violations } = await sizeViolations(page);
      expect(violations).toEqual([]);
    });

    test("onboarding", async ({ page }) => {
      await page.goto(
        `/tests/fixtures/settings-pages.html?lang=${language}&onboarding=1`,
      );
      await expect(page.locator("[data-control]").first()).toBeVisible();

      const { controls, violations } = await sizeViolations(page);
      expect(controls).toBeGreaterThan(0);
      expect(violations).toEqual([]);
    });
  });
}
