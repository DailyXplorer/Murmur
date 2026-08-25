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
});
