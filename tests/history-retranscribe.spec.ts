import { expect, test, type Page } from "@playwright/test";

const SHOTS = process.env.RETRANSCRIBE_SHOTS;

const openHistory = async (page: Page, query = "", entries = 3) => {
  await page.goto(`/tests/fixtures/settings-pages.html${query}`);
  await page
    .getByRole("navigation")
    .getByRole("button", { name: "History", exact: true })
    .click();
  await expect(page.getByRole("button", { name: "Delete entry" })).toHaveCount(
    entries,
  );
};

const callsTo = (page: Page, command: string) =>
  page.evaluate(
    (name) =>
      window.settingsPages.calls.filter(({ command }) => command === name),
    command,
  );

const firstEntry = (page: Page) =>
  page.locator("div.flex.flex-col.gap-3").first();

test("transcribe again in a language is for that run only", async ({
  page,
}) => {
  await openHistory(page);
  await firstEntry(page)
    .getByRole("button", { name: "Transcribe again in a chosen language" })
    .click();

  const menu = page.getByRole("listbox", { name: "Language for this run" });
  await expect(menu.getByRole("option", { name: "Auto" })).toBeVisible();
  await expect(menu.getByRole("option", { name: "French" })).toBeVisible();
  await expect(menu.getByRole("option", { name: "Japanese" })).toBeVisible();
  if (SHOTS) {
    await page.screenshot({ path: `${SHOTS}/language-menu-cloud.png` });
  }

  await menu.getByRole("option", { name: "French" }).click();
  await expect(menu).toHaveCount(0);

  const [call] = await callsTo(page, "retry_history_entry_transcription");
  expect(call.args).toMatchObject({ id: 3, language: "fr" });
  expect(await callsTo(page, "change_selected_language_setting")).toHaveLength(
    0,
  );
});

test("the plain button keeps using the language setting", async ({ page }) => {
  await openHistory(page);
  await firstEntry(page)
    .getByRole("button", { name: "Transcribe again", exact: true })
    .click();

  const [call] = await callsTo(page, "retry_history_entry_transcription");
  expect(call.args).toMatchObject({ id: 3, language: null });
});

test("the menu offers only what the engine on this computer transcribes", async ({
  page,
}) => {
  await openHistory(page, "?engine=local");
  await firstEntry(page)
    .getByRole("button", { name: "Transcribe again in a chosen language" })
    .click();

  const menu = page.getByRole("listbox", { name: "Language for this run" });
  await expect(menu.getByRole("option", { name: "French" })).toBeVisible();
  await expect(menu.getByRole("option", { name: "Japanese" })).toHaveCount(0);
  await expect(menu.getByRole("option", { name: "Auto" })).toBeVisible();
  if (SHOTS) {
    await page.screenshot({ path: `${SHOTS}/language-menu-local.png` });
  }
});

test("the menu explains itself in an info bubble, not inline", async ({
  page,
}) => {
  await openHistory(page);
  await firstEntry(page)
    .getByRole("button", { name: "Transcribe again in a chosen language" })
    .click();

  await expect(page.getByText("Used for this run only")).toHaveCount(0);
  await page
    .getByRole("button", {
      name: "Used for this run only. Your language setting doesn't change.",
    })
    .hover();
  await expect(
    page.getByRole("tooltip").getByText("Used for this run only"),
  ).toBeVisible();
});

test("each entry says where it was transcribed", async ({ page }) => {
  await openHistory(page, "?history=fallback", 6);
  await expect(page.getByText("ChatGPT", { exact: true })).toHaveCount(3);
  await expect(page.getByText("On this Mac", { exact: true })).toHaveCount(3);
});

test("earlier versions can be copied and restored", async ({ page }) => {
  await openHistory(page);
  const entry = firstEntry(page);
  await expect(
    page.getByRole("button", { name: "Earlier versions (2)" }),
  ).toHaveCount(1);
  await expect(page.getByText("ready for a review.")).toHaveCount(0);

  await entry.getByRole("button", { name: "Earlier versions (2)" }).click();
  await expect(page.getByText("ready for a review.")).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Restore this version" }),
  ).toHaveCount(2);
  if (SHOTS) {
    await page.screenshot({ path: `${SHOTS}/versions.png` });
  }

  await page
    .getByRole("button", { name: "Restore this version" })
    .first()
    .click();
  const [call] = await callsTo(page, "restore_history_version");
  expect(call.args).toMatchObject({ id: 3, versionId: 31 });
  await expect(page.getByText("Earlier version restored.")).toBeVisible();

  // The open list follows the swap: the text that was current is now the
  // first version, though the count is still 2.
  const list = entry.getByRole("list");
  await expect(list.getByText("ready for a review.")).toHaveCount(0);
  await expect(list.getByText("ready for review.")).toHaveCount(2);
  await expect(
    entry.getByText("The quarterly report is ready for a review."),
  ).toBeVisible();
});

test("the Windows build says this PC", async ({ page }) => {
  await openHistory(page, "?history=fallback&platform=windows", 6);
  await expect(page.getByText("On this PC", { exact: true })).toHaveCount(3);
  await expect(page.getByText("On this Mac", { exact: true })).toHaveCount(0);
  if (SHOTS) {
    await page.screenshot({ path: `${SHOTS}/engine-labels-windows.png` });
  }
});

test("the info bubble of the earlier versions sits on the line of its link", async ({
  page,
}) => {
  await openHistory(page);
  const link = firstEntry(page).getByRole("button", {
    name: "Earlier versions (2)",
  });
  const info = link.locator("..").getByRole("button").nth(1);
  const [linkBox, infoBox] = [
    await link.boundingBox(),
    await info.boundingBox(),
  ];
  const middle = (box: { y: number; height: number } | null) =>
    box ? box.y + box.height / 2 : Number.NaN;

  expect(Math.abs(middle(linkBox) - middle(infoBox))).toBeLessThanOrEqual(1);
  // The bubble's hit area must not make the line taller than its text.
  expect((await link.locator("..").boundingBox())?.height).toBe(
    linkBox?.height,
  );
});
