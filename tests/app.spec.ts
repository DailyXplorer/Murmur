import { test, expect } from "@playwright/test";

test.describe("Murmur App", () => {
  test("dev server responds", async ({ page }) => {
    // Just verify the dev server is running and responds
    const response = await page.goto("/");
    expect(response?.status()).toBe(200);
  });

  test("page has html structure", async ({ page }) => {
    await page.goto("/");

    // Verify basic HTML structure exists
    const html = await page.content();
    expect(html).toContain("<html");
    expect(html).toContain("<body");
  });

  test("logo fill follows every accent color", async ({ page }) => {
    await page.goto("/");

    const samples = await page.evaluate(async () => {
      const accents = ["pink", "blue", "green", "yellow", "orange", "red"];
      const logo = new Image();
      logo.src = "/src/assets/murmur-text-logo.png";
      await logo.decode();

      return accents.map((accent) => {
        document.documentElement.dataset.accentColor = accent;
        const styles = getComputedStyle(document.documentElement);
        const filter =
          styles.getPropertyValue("--accent-image-filter").trim() || "none";
        const expected = styles
          .getPropertyValue("--light-color-logo-primary")
          .trim();
        const canvas = document.createElement("canvas");
        canvas.width = logo.naturalWidth;
        canvas.height = logo.naturalHeight;
        const context = canvas.getContext("2d");

        if (!context) throw new Error("Canvas 2D context is unavailable");

        context.filter = filter;
        context.drawImage(logo, 0, 0);
        const [red, green, blue] = context.getImageData(200, 100, 1, 1).data;

        return { accent, actual: [red, green, blue], expected };
      });
    });

    for (const { accent, actual, expected } of samples) {
      const expectedChannels = expected
        .slice(1)
        .match(/.{2}/g)
        ?.map((channel) => Number.parseInt(channel, 16));

      expect(
        expectedChannels,
        `${accent} exposes a hex logo color`,
      ).toBeTruthy();
      expect(
        Math.max(
          ...actual.map((channel, index) =>
            Math.abs(channel - (expectedChannels?.[index] ?? channel)),
          ),
        ),
        `${accent} logo fill matches its accent token`,
      ).toBeLessThanOrEqual(12);
    }
  });

  test("solid surfaces follow every accent color with readable text", async ({
    page,
  }) => {
    await page.goto("/");

    const samples = await page.evaluate(() => {
      const accents = ["pink", "blue", "green", "yellow", "orange", "red"];
      const themes = ["light", "dark"];
      const properties = [
        "--color-logo-primary",
        "--color-background-ui",
        "--color-background-ui-hover",
        "--color-background-ui-active",
        "--color-on-accent",
      ];
      const canvas = document.createElement("canvas");
      canvas.width = 1;
      canvas.height = 1;
      const context = canvas.getContext("2d", { willReadFrequently: true });
      const probe = document.createElement("div");
      document.body.appendChild(probe);

      if (!context) throw new Error("Canvas 2D context is unavailable");

      const resolveColor = (property: string): number[] => {
        probe.style.backgroundColor = `var(${property})`;
        context.clearRect(0, 0, 1, 1);
        context.fillStyle = "#000000";
        context.fillStyle = getComputedStyle(probe).backgroundColor;
        context.fillRect(0, 0, 1, 1);
        return Array.from(context.getImageData(0, 0, 1, 1).data.slice(0, 3));
      };

      const results = themes.flatMap((theme) => {
        document.documentElement.dataset.theme = theme;

        return accents.map((accent) => {
          document.documentElement.dataset.accentColor = accent;
          const colors = Object.fromEntries(
            properties.map((property) => [property, resolveColor(property)]),
          );

          return { accent, theme, colors };
        });
      });

      probe.remove();
      return results;
    });

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
    const largestChannelDifference = (
      first: number[],
      second: number[],
    ): number =>
      Math.max(
        ...first.map((channel, index) => Math.abs(channel - second[index])),
      );

    for (const { accent, theme, colors } of samples) {
      const logo = colors["--color-logo-primary"];
      const resting = colors["--color-background-ui"];
      const hover = colors["--color-background-ui-hover"];
      const active = colors["--color-background-ui-active"];
      const foreground = colors["--color-on-accent"];
      const label = `${accent} in ${theme} mode`;
      const hoverDifference = largestChannelDifference(resting, hover);
      const activeDifference = largestChannelDifference(resting, active);

      expect(resting, `${label} uses the active logo color`).toEqual(logo);
      for (const [state, background] of [
        ["resting", resting],
        ["hover", hover],
        ["pressed", active],
      ] as const) {
        expect(
          contrastRatio(background, foreground),
          `${label} keeps ${state} text contrast above 4.5:1`,
        ).toBeGreaterThanOrEqual(4.5);
      }
      expect(
        hoverDifference,
        `${label} has a visible hover state`,
      ).toBeGreaterThan(0);
      expect(
        activeDifference,
        `${label} has a stronger pressed state than hover`,
      ).toBeGreaterThan(hoverDifference);
      expect(
        activeDifference,
        `${label} keeps the pressed state close to the selected color`,
      ).toBeLessThanOrEqual(32);
    }
  });
});
