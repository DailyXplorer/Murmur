import { expect, test, type Locator, type Page } from "@playwright/test";
import { DEFAULT_WINDOW, MIN_WINDOW } from "./windowSizes";

/** Set to a folder to save a screenshot of every open menu there. */
const SHOTS = process.env.LANGUAGE_MENU_SHOTS;

const SIDES = ["light", "dark"] as const;
type Theme = (typeof SIDES)[number];

const openHistory = async (page: Page, theme: Theme = "light") => {
  await page.emulateMedia({ colorScheme: theme });
  await page.goto(`/tests/fixtures/settings-pages.html?theme=${theme}`);
  await page
    .getByRole("navigation")
    .getByRole("button", { name: "History", exact: true })
    .click();
  await expect(page.getByRole("button", { name: "Delete entry" })).toHaveCount(
    3,
  );
};

const caret = (page: Page, index: number) =>
  page
    .getByRole("button", { name: "Transcribe again in a chosen language" })
    .nth(index);

/** The portaled panel: the listbox's parent, which carries the position. */
const panelOf = (page: Page) => page.locator("[data-floating-panel-root]");

/** Fails when any part of `box` lies outside the viewport. */
const expectInsideViewport = async (page: Page, panel: Locator) => {
  const viewport = page.viewportSize();
  if (!viewport) throw new Error("No viewport");
  const box = await panel.boundingBox();
  if (!box) throw new Error("The menu has no box");
  expect(box.x).toBeGreaterThanOrEqual(0);
  expect(box.y).toBeGreaterThanOrEqual(0);
  expect(box.x + box.width).toBeLessThanOrEqual(viewport.width);
  expect(box.y + box.height).toBeLessThanOrEqual(viewport.height);
};

/** True when a text node is cut by `overflow` or an ellipsis. */
const isTruncated = (locator: Locator) =>
  locator.evaluate((element) => element.scrollWidth > element.clientWidth);

const widths = [
  { name: "minimum", width: MIN_WINDOW.width },
  { name: "default", width: DEFAULT_WINDOW.width },
  { name: "narrower than the minimum", width: 420 },
];

for (const { name, width } of widths) {
  test.describe(`language menu in a ${name} window (${width} px)`, () => {
    test.beforeEach(async ({ page }) => {
      await page.setViewportSize({ width, height: MIN_WINDOW.height });
    });

    for (const row of [0, 1, 2]) {
      test(`stays inside the window on row ${row + 1}`, async ({ page }) => {
        await openHistory(page);
        await caret(page, row).scrollIntoViewIfNeeded();
        await caret(page, row).click();

        const menu = page.getByRole("listbox", {
          name: "Language for this run",
        });
        await expect(menu.getByRole("option", { name: "Auto" })).toBeVisible();
        const panel = panelOf(page);
        await expect(panel).toBeVisible();
        await expectInsideViewport(page, panel);

        // The title and every language name read in full.
        const title = panel.getByRole("heading", {
          name: "Language for this run",
        });
        await expect(title).toBeVisible();
        expect(await isTruncated(title)).toBe(false);
        const options = menu.getByRole("option");
        const count = await options.count();
        expect(count).toBeGreaterThan(5);
        for (let index = 0; index < count; index += 1) {
          expect(await isTruncated(options.nth(index).locator("span"))).toBe(
            false,
          );
        }

        // The last language is reachable by scrolling the list, and the panel
        // is still inside the window afterwards.
        await options.last().scrollIntoViewIfNeeded();
        await expect(options.last()).toBeInViewport();
        await expectInsideViewport(page, panel);
      });
    }
  });
}

test("the menu follows a window that is resized while it is open", async ({
  page,
}) => {
  await page.setViewportSize({
    width: DEFAULT_WINDOW.width,
    height: MIN_WINDOW.height,
  });
  await openHistory(page);
  await caret(page, 0).click();
  const panel = panelOf(page);
  await expect(panel).toBeVisible();
  await expectInsideViewport(page, panel);

  await page.setViewportSize({
    width: MIN_WINDOW.width,
    height: MIN_WINDOW.height,
  });
  await expect
    .poll(async () => {
      const box = await panel.boundingBox();
      return box ? box.x + box.width : Infinity;
    })
    .toBeLessThanOrEqual(MIN_WINDOW.width);
  await expectInsideViewport(page, panel);
});

test("the menu ends where its button ends when there is room", async ({
  page,
}) => {
  await page.setViewportSize({
    width: DEFAULT_WINDOW.width,
    height: MIN_WINDOW.height,
  });
  await openHistory(page);
  const trigger = caret(page, 0);
  await trigger.click();
  const [menuBox, triggerBox] = await Promise.all([
    panelOf(page).boundingBox(),
    trigger.boundingBox(),
  ]);
  if (!menuBox || !triggerBox) throw new Error("Expected visible boxes");
  expect(menuBox.x + menuBox.width).toBeCloseTo(
    triggerBox.x + triggerBox.width,
    0,
  );
});

for (const theme of SIDES) {
  test(`screenshot of the menu on the last row, ${theme}`, async ({ page }) => {
    test.skip(!SHOTS, "Set LANGUAGE_MENU_SHOTS to save screenshots");
    await page.setViewportSize({
      width: MIN_WINDOW.width,
      height: MIN_WINDOW.height,
    });
    await openHistory(page, theme);
    await caret(page, 2).scrollIntoViewIfNeeded();
    await caret(page, 2).click();
    await expect(panelOf(page)).toBeVisible();
    await expect(page.getByRole("option", { name: "Auto" })).toBeVisible();
    await page.waitForTimeout(250);
    await page.screenshot({
      path: `${SHOTS}/language-menu-min-width-last-row-${theme}.png`,
    });

    await caret(page, 0).scrollIntoViewIfNeeded();
    await page.keyboard.press("Escape");
    await caret(page, 0).click();
    await expect(panelOf(page)).toBeVisible();
    await page.waitForTimeout(250);
    await page.screenshot({
      path: `${SHOTS}/language-menu-min-width-first-row-${theme}.png`,
    });
  });
}

// The other floating things added in the same release: they follow the same
// rule, so a small window never cuts them.
test("the Insights share window fits the minimum window", async ({ page }) => {
  await page.setViewportSize(MIN_WINDOW);
  await page.goto("/tests/fixtures/insights.html");
  await page.waitForFunction(() => window.insights?.ready === true);
  await page.getByRole("button", { name: "Share", exact: true }).click();
  const dialog = page.getByRole("dialog", { name: "Share your insights" });
  await expect(dialog).toBeVisible();
  await expect(
    dialog.getByRole("button", { name: "Copy image" }),
  ).toBeVisible();
  await expectInsideViewport(page, dialog);
});

test("the capture mode menu of Meetings stays inside the minimum window", async ({
  page,
}) => {
  await page.setViewportSize(MIN_WINDOW);
  await page.goto("/tests/fixtures/settings-pages.html?meetings=1");
  await page
    .getByRole("navigation")
    .getByRole("button", { name: "Meetings", exact: true })
    .click();
  const capture = page
    .locator("[aria-haspopup='listbox']")
    .filter({ visible: true })
    .first();
  await expect(capture).toBeVisible();
  await capture.scrollIntoViewIfNeeded();
  await capture.click();
  await expect(page.getByRole("listbox")).toBeVisible();
  await expectInsideViewport(page, panelOf(page));
});
