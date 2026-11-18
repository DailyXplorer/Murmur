import { expect, test, type Locator, type Page } from "@playwright/test";
import { readdirSync, readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { DEFAULT_WINDOW, MIN_WINDOW } from "./windowSizes";

const fixture = "/tests/fixtures/settings-pages.html";
const outputDir = process.env.MURMUR_SCREENSHOT_DIR;

const PROJECT_URL = "https://github.com/DailyXplorer/Murmur";
const DESCRIPTION =
  "A star on GitHub helps other people find Murmur. This opens the project page, where you click Star.";
const SECTIONS = [
  "General",
  "Transcription",
  "History",
  "Meetings",
  "Insights",
  "Advanced",
  "Debug",
  "About",
];

const localesDir = fileURLToPath(
  new URL("../src/i18n/locales/", import.meta.url),
);
const LOCALES = readdirSync(localesDir);
const sidebarLabel = (locale: string) =>
  (
    JSON.parse(
      readFileSync(`${localesDir}${locale}/translation.json`, "utf8"),
    ) as { sidebar: { star: string } }
  ).sidebar.star;

// The app runs in WKWebView and WebView2, not in this Chromium, so a label
// that fits here by a fraction of a pixel may be cut there.
const SPARE_PX = 2;

const sidebarButton = (page: Page, name: string) =>
  page.getByRole("navigation").getByRole("button", { name, exact: true });

const star = (page: Page, locale = "en") =>
  sidebarButton(page, sidebarLabel(locale));

const openedUrls = (page: Page) =>
  page.evaluate(() =>
    window.settingsPages.calls
      .filter(({ command }) => command === "plugin:opener|open_url")
      .map(({ args }) => (args as { url: string }).url),
  );

const box = async (locator: Locator) => {
  const value = await locator.boundingBox();
  if (value === null) throw new Error("element is not visible");
  return value;
};

test("the last About row asks for a star and opens the project page", async ({
  page,
}) => {
  await page.goto(fixture);
  await sidebarButton(page, "About").click();
  const about = page.getByRole("main", { name: "About" });

  await expect(about.getByRole("heading", { level: 3 }).last()).toHaveText(
    "Like Murmur?",
  );
  await about.getByRole("button", { name: DESCRIPTION, exact: true }).hover();
  await expect(page.getByRole("tooltip")).toHaveText(DESCRIPTION);

  expect(await openedUrls(page)).toEqual([]);
  await about.getByRole("button", { name: "Give a star", exact: true }).click();
  await expect.poll(() => openedUrls(page)).toEqual([PROJECT_URL]);

  await about
    .getByRole("button", { name: "View on GitHub", exact: true })
    .click();
  await expect.poll(() => openedUrls(page)).toEqual([PROJECT_URL, PROJECT_URL]);
});

test("the sidebar button sits under the sections on every page and opens the project page", async ({
  page,
}) => {
  await page.setViewportSize(MIN_WINDOW);
  await page.goto(fixture);
  const button = star(page);

  for (const section of SECTIONS) {
    await sidebarButton(page, section).click();
    await expect(page.getByRole("heading", { level: 1 })).toHaveText(section);
    await expect(button, section).toBeInViewport({ ratio: 1 });
  }

  const [sidebar, last, own] = await Promise.all([
    box(page.getByRole("navigation")),
    box(sidebarButton(page, SECTIONS[SECTIONS.length - 1])),
    box(button),
  ]);
  expect(own.y).toBeGreaterThanOrEqual(last.y + last.height);
  const under = sidebar.y + sidebar.height - (own.y + own.height);
  expect(under).toBeGreaterThan(0);
  expect(under).toBeCloseTo(own.x - sidebar.x, 0);

  expect(await openedUrls(page)).toEqual([]);
  await button.click();
  await expect.poll(() => openedUrls(page)).toEqual([PROJECT_URL]);
  await expect(button).not.toHaveAttribute("aria-current");
  await expect(sidebarButton(page, "About")).toHaveAttribute(
    "aria-current",
    "page",
  );
  await expect(page.getByRole("heading", { level: 1 })).toHaveText("About");
});

test("a short Windows window scrolls its sidebar to the button", async ({
  page,
}) => {
  await page.setViewportSize({ width: 666, height: 313 });
  await page.goto(`${fixture}?platform=windows`);
  const button = star(page);

  await expect(sidebarButton(page, "General")).toBeInViewport({ ratio: 1 });
  await expect(button).not.toBeInViewport();
  await button.scrollIntoViewIfNeeded();
  await expect(button).toBeInViewport({ ratio: 1 });

  await button.click();
  await expect.poll(() => openedUrls(page)).toEqual([PROJECT_URL]);
});

for (const [size, viewport] of [
  ["default", DEFAULT_WINDOW],
  ["minimum", MIN_WINDOW],
] as const) {
  for (const locale of LOCALES) {
    test(`the sidebar label is whole in ${locale} at the ${size} window size`, async ({
      page,
    }) => {
      await page.setViewportSize(viewport);
      await page.goto(`${fixture}?lang=${locale}`);
      const label = star(page, locale).locator("span");
      await expect(label).toBeVisible();
      await page.evaluate(async () => {
        await document.fonts.ready;
      });

      const { cut, spare } = await label.evaluate((span) => {
        const button = span.closest("button")!;
        const inner =
          button.getBoundingClientRect().right -
          parseFloat(getComputedStyle(button).paddingRight);
        return {
          cut: span.scrollWidth > span.clientWidth,
          spare: inner - span.getBoundingClientRect().right,
        };
      });
      expect(cut).toBe(false);
      expect(spare).toBeGreaterThanOrEqual(SPARE_PX);
    });
  }
}

test.describe("screenshots", () => {
  test.use({ viewport: DEFAULT_WINDOW, deviceScaleFactor: 2 });

  for (const theme of ["light", "dark"] as const) {
    test(`the About page (${theme})`, async ({ page }) => {
      test.skip(!outputDir, "Set MURMUR_SCREENSHOT_DIR to capture screenshots");
      await page.emulateMedia({ colorScheme: theme });
      await page.goto(`${fixture}?theme=${theme}`);
      await sidebarButton(page, "About").click();
      await expect(
        page.getByRole("heading", { name: "Like Murmur?", exact: true }),
      ).toBeVisible();
      // The sidebar's selection slides over to About.
      await page.waitForTimeout(500);
      await page.screenshot({ path: `${outputDir}/about-star-${theme}.png` });
    });
  }
});
