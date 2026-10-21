import { expect, test, type Page } from "@playwright/test";

const fixture = "/tests/fixtures/accent-text.html";
const accents = ["pink", "blue", "green", "yellow", "orange", "red"];
const themes = ["light", "dark"];

const luminance = ([red, green, blue]: number[]): number => {
  const channels = [red, green, blue].map((channel) => {
    const value = channel / 255;
    return value <= 0.04045
      ? value / 12.92
      : Math.pow((value + 0.055) / 1.055, 2.4);
  });

  return 0.2126 * channels[0] + 0.7152 * channels[1] + 0.0722 * channels[2];
};

const contrastRatio = (first: number[], second: number[]): number => {
  const lighter = Math.max(luminance(first), luminance(second));
  const darker = Math.min(luminance(first), luminance(second));
  return (lighter + 0.05) / (darker + 0.05);
};

interface Sample {
  accent: string;
  theme: string;
  name: string;
  foreground: number[];
  background: number[];
}

/**
 * For every accent and theme, the colour `pick` reads from each element of
 * `selector`, painted over the backgrounds of that element and its ancestors.
 */
const sample = (
  page: Page,
  selector: string,
  pick: "text" | "ring",
): Promise<Sample[]> =>
  page.evaluate(
    ({ selector, pick, accents, themes }) => {
      const canvas = document.createElement("canvas");
      canvas.width = 1;
      canvas.height = 1;
      const context = canvas.getContext("2d", { willReadFrequently: true });
      if (!context) throw new Error("Canvas 2D context is unavailable");

      const paint = (layers: string[]): number[] => {
        context.clearRect(0, 0, 1, 1);
        for (const layer of layers) {
          context.fillStyle = "#000000";
          context.fillStyle = layer;
          context.fillRect(0, 0, 1, 1);
        }
        return Array.from(context.getImageData(0, 0, 1, 1).data.slice(0, 3));
      };

      const backgrounds = (element: Element | null): string[] => {
        const layers: string[] = [];
        for (let node = element; node; node = node.parentElement) {
          layers.unshift(getComputedStyle(node).backgroundColor);
        }
        return layers;
      };

      /** The colour of the widest visible box-shadow, the focus ring. */
      const ringColor = (element: Element): string => {
        const shadows = getComputedStyle(element).boxShadow.matchAll(
          /(rgba?\([^)]*\)|color\([^)]*\)) (-?[\d.]+)px (-?[\d.]+)px (-?[\d.]+)px (-?[\d.]+)px/g,
        );
        const visible = Array.from(shadows).filter(
          ([, color]) => !/rgba\(0, 0, 0, 0\)/.test(color),
        );
        if (visible.length === 0) throw new Error("No focus ring is drawn");
        visible.sort((a, b) => Number(b[5]) - Number(a[5]));
        return visible[0][1];
      };

      const elements = Array.from(document.querySelectorAll(selector));
      if (elements.length === 0) throw new Error(`Nothing matches ${selector}`);

      return themes.flatMap((theme) => {
        document.documentElement.dataset.theme = theme;
        return accents.flatMap((accent) => {
          document.documentElement.dataset.accentColor = accent;
          return elements.map((element) => {
            const below =
              pick === "text"
                ? backgrounds(element)
                : backgrounds(element.parentElement);
            const color =
              pick === "text"
                ? getComputedStyle(element).color
                : ringColor(element);
            return {
              accent,
              theme,
              name:
                element.getAttribute("aria-label") ??
                element.textContent?.trim() ??
                selector,
              foreground: paint([...below, color]),
              background: paint(below),
            };
          });
        });
      });
    },
    { selector, pick, accents, themes },
  );

const expectContrast = (samples: Sample[], minimum: number) => {
  for (const { accent, theme, name, foreground, background } of samples) {
    expect(
      contrastRatio(foreground, background),
      `"${name}" with the ${accent} accent in ${theme} mode`,
    ).toBeGreaterThanOrEqual(minimum);
  }
};

test("accent text reads at 4.5:1 for every accent in both themes", async ({
  page,
}) => {
  await page.goto(fixture);
  await page.locator('[aria-haspopup="listbox"]').click();
  await expect(page.getByRole("option", { selected: true })).toBeVisible();

  const selectors = [
    '[data-testid="markdown"] a',
    '[data-testid="speakers"] li',
    'button[aria-label="Pinned"]',
    '[role="option"][aria-selected="true"]',
  ];
  for (const selector of selectors) {
    expectContrast(await sample(page, selector, "text"), 4.5);
  }
});

test("focus rings stand out at 3:1 for every accent in both themes", async ({
  page,
}) => {
  await page.goto(fixture);

  for (const name of ["Focus me", "Pinned"]) {
    await page.keyboard.press("Tab");
    const focused = page.getByRole("button", { name, exact: true });
    await expect(focused).toBeFocused();
    expect(
      await focused.evaluate((element) => element.matches(":focus-visible")),
    ).toBe(true);
    expectContrast(await sample(page, "button:focus", "ring"), 3);
  }
});
