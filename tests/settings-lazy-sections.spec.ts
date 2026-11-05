import { expect, test, type Page } from "@playwright/test";

const fixture = "/tests/fixtures/settings-pages.html";
const outputDir = process.env.MURMUR_SCREENSHOT_DIR;

const open = async (page: Page, section: string) => {
  await page
    .getByRole("navigation")
    .getByRole("button", { name: section, exact: true })
    .click();
};

// Each loads with its first open, after General painted.
const LAZY_SECTIONS = [
  {
    name: "History",
    file: "history",
    shown: (page: Page) => page.getByRole("button", { name: "Delete entry" }),
  },
  {
    name: "Meetings",
    file: "meetings",
    shown: (page: Page) => page.getByText("Point hebdo produit").first(),
  },
  {
    name: "Insights",
    file: "insights",
    shown: (page: Page) => page.getByRole("heading", { name: "Time saved" }),
  },
];

for (const section of LAZY_SECTIONS) {
  test(`${section.name} renders when first opened`, async ({ page }) => {
    await page.goto(fixture);
    await expect(
      page.getByRole("navigation").getByRole("button", { name: "General" }),
    ).toBeVisible();

    await open(page, section.name);

    await expect(section.shown(page).first()).toBeVisible();
    expect(
      await page.evaluate(() => window.settingsPages.unhandledCommands),
    ).toEqual([]);
    if (outputDir) {
      await page.screenshot({
        path: `${outputDir}/lazy-${section.file}.png`,
        fullPage: true,
      });
    }
  });
}

test("a reopened window shows the section left last", async ({ page }) => {
  await page.goto(`${fixture}?lastSection=history`);

  await expect(
    page.getByRole("button", { name: "Delete entry" }).first(),
  ).toBeVisible();
  await expect(
    page
      .getByRole("navigation")
      .getByRole("button", { name: "History", exact: true }),
  ).toHaveAttribute("aria-current", "page");
});

test("the window reports the section it shows", async ({ page }) => {
  await page.goto(fixture);
  await open(page, "Insights");
  await expect(page.getByRole("heading", { name: "Time saved" })).toBeVisible();

  const views = await page.evaluate(() =>
    window.settingsPages.calls
      .filter(({ command }) => command === "report_settings_view")
      .map(({ args }) => args),
  );
  expect(views.at(-1)).toEqual({ view: { section: "insights" } });
});

test("a check asked from the tray while the window was closed runs once it opens", async ({
  page,
}) => {
  await page.goto(`${fixture}?pending=check_updates`);

  await expect
    .poll(() =>
      page.evaluate(
        () =>
          window.settingsPages.calls.filter(
            ({ command }) => command === "check_for_updates",
          ).length,
      ),
    )
    .toBe(1);
});

test("a switch to this Mac made while the window was closed is told once", async ({
  page,
}) => {
  await page.goto(`${fixture}?pending=switched_to_local`);

  await expect(page.getByText("Now transcribing on this Mac.")).toHaveCount(1);
});
