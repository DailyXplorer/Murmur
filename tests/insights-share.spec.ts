import { expect, test, type Page } from "@playwright/test";

const SHOTS = process.env.INSIGHTS_SCREENSHOT_DIR;

const openCard = async (page: Page, query: string) => {
  await page.setViewportSize({ width: 1200, height: 1200 });
  await page.goto(`/tests/fixtures/share-card.html${query}`);
  await page.waitForFunction(() => window.shareCard?.ready === true);
  return page.locator("#frame > div");
};

const openInsights = async (page: Page, query = "") => {
  await page.setViewportSize({ width: 900, height: 1000 });
  await page.goto(`/tests/fixtures/insights.html${query}`);
  await page.waitForFunction(() => window.insights?.ready === true);
};

/** The pixel size and first bytes of a base64 PNG. */
const inspectPng = async (page: Page, png: string) =>
  page.evaluate(async (base64) => {
    const bytes = Uint8Array.from(atob(base64), (c) => c.charCodeAt(0));
    const bitmap = await createImageBitmap(new Blob([bytes]));
    return {
      width: bitmap.width,
      height: bitmap.height,
      signature: Array.from(bytes.slice(0, 4)),
    };
  }, png);

/** Whether the text of an element runs past the box it sits in. */
const overflowing = (card: ReturnType<Page["locator"]>) =>
  card.evaluate((root) => {
    const bad: string[] = [];
    for (const el of root.querySelectorAll<HTMLElement>("[data-fit]")) {
      const room = el.parentElement!.getBoundingClientRect();
      const box = el.getBoundingClientRect();
      if (box.right > room.right + 0.5) bad.push(el.textContent ?? "");
    }
    const rootBox = root.getBoundingClientRect();
    for (const el of root.querySelectorAll<HTMLElement>("*")) {
      const box = el.getBoundingClientRect();
      if (
        box.width &&
        (box.right > rootBox.right + 0.5 || box.bottom > rootBox.bottom + 0.5)
      ) {
        if (el.tagName !== "IMG") bad.push(`${el.tagName}:${el.textContent}`);
      }
    }
    return bad;
  });

for (const data of ["demo", "huge", "low"] as const) {
  test(`the ${data} card fits its 1080 px square`, async ({ page }) => {
    const card = await openCard(page, `?data=${data}`);
    await expect(card).toHaveCSS("width", "1080px");
    await expect(card).toHaveCSS("height", "1080px");
    expect(await overflowing(card)).toEqual([]);
    if (SHOTS) await card.screenshot({ path: `${SHOTS}/card-${data}.png` });
  });
}

test("the card is light in a dark app and shows the numbers it is given", async ({
  page,
}) => {
  const card = await openCard(page, "?data=demo&theme=dark");
  await expect(card).toHaveCSS("background-color", "rgb(251, 251, 251)");
  await expect(card).toContainText("Words dictated");
  await expect(card).toContainText("41,260");
  await expect(card).toContainText("12 h 12 min");
  await expect(card).toContainText("138");
  await expect(card).toContainText("23 days");
  await expect(card).toContainText("58 days");
  await expect(card).toContainText(
    "Dictated with Murmur — murmur-desktop.vercel.app",
  );
});

test("a huge number shrinks to fit and keeps the layout", async ({ page }) => {
  const card = await openCard(page, "?data=huge");
  const size = await card
    .locator('[data-fit="hero"]')
    .evaluate((el) => parseFloat(getComputedStyle(el).fontSize));
  expect(size).toBeLessThan(250);
  expect(size).toBeGreaterThan(60);
  await expect(card).toContainText("1,234,567,890");
});

test("a new user's card shows zeros and a dash, not blanks", async ({
  page,
}) => {
  const card = await openCard(page, "?data=low");
  await expect(card).toContainText("0 min");
  await expect(card.locator('[data-fit="tile-value"]').nth(1)).toHaveText("–");
});

for (const [lng, words, url] of [
  ["fr", "Mots dictés", "Dicté avec"],
  ["es", "Palabras dictadas", "Dictado con"],
  ["de", "Diktierte Wörter", "Diktiert mit"],
  ["it", "Parole dettate", "Dettato con"],
  ["pt", "Palavras ditadas", "Ditado com"],
] as const) {
  test(`the card is in ${lng}`, async ({ page }) => {
    const card = await openCard(page, `?data=huge&lng=${lng}`);
    await expect(card).toContainText(words);
    await expect(card).toContainText(url);
    expect(await overflowing(card)).toEqual([]);
    if (SHOTS) await card.screenshot({ path: `${SHOTS}/card-huge-${lng}.png` });
  });
}

test("the export is a 1080 x 1080 PNG whatever the pixel ratio", async ({
  browser,
}) => {
  const context = await browser.newContext({ deviceScaleFactor: 2 });
  const page = await context.newPage();
  await openCard(page, "?data=demo");
  const png = await page.evaluate(() => window.shareCard.exportPng());
  expect(await inspectPng(page, png)).toEqual({
    width: 1080,
    height: 1080,
    signature: [0x89, 0x50, 0x4e, 0x47],
  });
  if (SHOTS) {
    const { writeFileSync } = await import("node:fs");
    writeFileSync(`${SHOTS}/export-demo.png`, Buffer.from(png, "base64"));
    const huge = await openCard(page, "?data=huge");
    void huge;
    writeFileSync(
      `${SHOTS}/export-huge.png`,
      Buffer.from(
        await page.evaluate(() => window.shareCard.exportPng()),
        "base64",
      ),
    );
  }
  await context.close();
});

test("Share opens the card, and Save and Copy send its PNG to the app", async ({
  page,
}) => {
  await openInsights(page);
  const share = page.getByRole("button", { name: "Share", exact: true });
  await expect(share).toBeEnabled();
  if (SHOTS)
    await page.screenshot({ path: `${SHOTS}/insights-share-button.png` });

  await share.click();
  const dialog = page.getByRole("dialog", { name: "Share your insights" });
  await expect(dialog).toBeVisible();
  await expect(dialog.getByText("48,210")).toBeVisible();
  // No options: two actions and a close button, nothing else.
  await expect(dialog.getByRole("button", { name: "Close" })).toBeVisible();
  await expect(dialog.getByRole("button")).toHaveCount(3);
  await expect(dialog.getByRole("button", { name: "Save PNG" })).toBeVisible();
  await expect(
    dialog.getByRole("button", { name: "Copy image" }),
  ).toBeVisible();
  await expect(dialog.getByRole("checkbox")).toHaveCount(0);
  await expect(dialog.getByRole("switch")).toHaveCount(0);
  if (SHOTS)
    await page.screenshot({ path: `${SHOTS}/insights-share-dialog.png` });

  await dialog.getByRole("button", { name: "Save PNG" }).click();
  await expect(page.getByText("Image saved.")).toBeVisible();
  await dialog.getByRole("button", { name: "Copy image" }).click();
  await expect(page.getByText("Image copied.")).toBeVisible();

  const sent = await page.evaluate(() => window.insights.shared());
  expect(sent.map((s) => s.command)).toEqual([
    "save_insights_card",
    "copy_insights_card",
  ]);
  for (const { png } of sent) {
    expect(await inspectPng(page, png)).toMatchObject({
      width: 1080,
      height: 1080,
    });
  }
  // The picture holds numbers only: its bytes are the same for both.
  expect(sent[0].png.length).toBeGreaterThan(10_000);
});

test("Share is a standard-width button after the Streak section", async ({
  page,
}) => {
  await openInsights(page);
  const share = page.getByRole("button", { name: "Share", exact: true });
  const streak = page.getByRole("heading", { name: "Streak", exact: true });
  const main = page.getByRole("main", { name: "Insights" });
  const [shareBox, streakBox, mainBox, tileBox] = await Promise.all([
    share.boundingBox(),
    streak.locator("xpath=ancestor::section[1]").boundingBox(),
    main.boundingBox(),
    page
      .locator("section")
      .filter({ hasText: "Total words" })
      .first()
      .boundingBox(),
  ]);
  const shared = await share.evaluate((el) =>
    parseFloat(getComputedStyle(el).minWidth),
  );
  expect(shared).toBeGreaterThan(60);
  expect(shareBox!.width).toBeCloseTo(shared, 1);
  // Below the whole Streak section, flush with the right edge of the content.
  expect(shareBox!.y).toBeGreaterThanOrEqual(
    streakBox!.y + streakBox!.height - 0.5,
  );
  expect(shareBox!.x + shareBox!.width).toBeCloseTo(
    mainBox!.x + mainBox!.width,
    0,
  );
  // Nothing sits above the stat cards: they start the page.
  expect(tileBox!.y - mainBox!.y).toBeLessThan(2);
});

test("cancelling the save dialog says nothing", async ({ page }) => {
  await openInsights(page, "?save=cancel");
  await page.getByRole("button", { name: "Share", exact: true }).click();
  await page.getByRole("button", { name: "Save PNG" }).click();
  await expect
    .poll(() => page.evaluate(() => window.insights.shared().length))
    .toBe(1);
  await expect(page.getByText("Image saved.")).toHaveCount(0);
  await expect(page.getByText("The image could not be saved.")).toHaveCount(0);
});

test("with nothing dictated yet, Share is off and says why", async ({
  page,
}) => {
  await openInsights(page, "?data=empty");
  const share = page.getByRole("button", { name: "Share", exact: true });
  await expect(share).toBeDisabled();
  await expect(
    page.getByTitle("Dictate something first, then share your insights."),
  ).toBeVisible();
});
