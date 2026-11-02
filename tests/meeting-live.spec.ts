import { expect, test, type Page } from "@playwright/test";
import type { LiveEntry, LiveState } from "../src/bindings";
import { DEFAULT_WINDOW } from "./windowSizes";

const fixture =
  "/tests/fixtures/settings-pages.html?meetings=1&meeting=recording";
const outputDir = process.env.MURMUR_SCREENSHOT_DIR;

const openLive = async (page: Page, query = "") => {
  await page.goto(`${fixture}${query}`);
  await page
    .getByRole("navigation")
    .getByRole("button", { name: "Meetings", exact: true })
    .click();
  await page.getByRole("listitem").nth(0).getByRole("button").first().click();
  await expect(
    page.getByRole("heading", { name: "Live transcript" }),
  ).toBeVisible();
};

const said = (
  source: LiveEntry["source"],
  start_s: number,
  text: string,
): LiveEntry => ({
  end_ms: (start_s + 3) * 1000,
  kind: "speech",
  source,
  start_ms: start_s * 1000,
  text,
});

const CONVERSATION: LiveEntry[] = [
  said("system", 2, "Bonjour à tous, on commence par le point produit ?"),
  said("mic", 6, "Oui. La nouvelle page d'accueil est prête pour jeudi."),
  said("mic", 10, "Il reste deux écrans à finaliser."),
  said("system", 15, "Parfait, merci."),
];

const publish = (page: Page, state: LiveState, appended: LiveEntry[] = []) =>
  page.evaluate((update) => window.settingsPages.emitLive(update), {
    appended,
    id: 4,
    state,
  });

const callsTo = (page: Page, command: string) =>
  page.evaluate(
    (name) =>
      window.settingsPages.calls.filter((call) => call.command === name).length,
    command,
  );

test("the recording meeting opens its live view, listening until someone speaks", async ({
  page,
}) => {
  await openLive(page);
  await expect(
    page.getByText(
      "Listening… text appears a few seconds after someone speaks.",
    ),
  ).toBeVisible();
  await expect(page.getByText(/^Recording · 12:3\d$/)).toBeVisible();
  await expect(page.getByRole("button", { name: "Copy all" })).toBeDisabled();
  await expect(page.getByText(/pasting new meeting text/)).toBeVisible();
  expect(
    await page.evaluate(() => window.settingsPages.unhandledCommands),
  ).toEqual([]);
});

test("live sentences show in time order, labelled You and Others at each change", async ({
  page,
}) => {
  await openLive(page);
  await publish(page, { state: "live" }, CONVERSATION.slice(1, 3));
  await publish(page, { state: "live" }, [CONVERSATION[0], CONVERSATION[3]]);

  const items = page
    .getByRole("list", { name: "Live transcript" })
    .getByRole("listitem");
  await expect(items).toHaveCount(4);
  await expect(items.nth(0)).toHaveText(
    "OthersBonjour à tous, on commence par le point produit ?",
  );
  await expect(items.nth(1)).toHaveText(
    "YouOui. La nouvelle page d'accueil est prête pour jeudi.",
  );
  await expect(items.nth(2)).toHaveText("Il reste deux écrans à finaliser.");
  await expect(items.nth(3)).toHaveText("OthersParfait, merci.");
  await expect(page.getByText(/^Listening/)).toHaveCount(0);
  await expect(page.getByRole("button", { name: "Copy all" })).toBeEnabled();

  await publish(page, { state: "catching_up" }, [
    { ...said("system", 20, ""), end_ms: 140_000, kind: "gap" },
  ]);
  await expect(
    page.getByRole("status").filter({ hasText: "Catching up" }),
  ).toBeVisible();
  await expect(
    page.getByText("… (gap, the final transcript will include it)"),
  ).toBeVisible();
  await publish(page, { state: "live" });
  await expect(page.getByText("Catching up")).toHaveCount(0);
});

test("without the on-device model the live view says so and offers the download", async ({
  page,
}) => {
  await openLive(page, "&models=missing&live=missing");
  const notice = page.getByRole("status").filter({
    hasText: "Live text needs the on-device speech model.",
  });
  await expect(notice.locator("p")).toHaveCount(1);
  await expect(page.getByText(/^Listening/)).toHaveCount(0);

  await notice.getByRole("button", { name: "Download" }).click();
  await expect.poll(() => callsTo(page, "download_local_model")).toBe(1);
  await expect(notice).toContainText("Downloading");
});

test("three failed pieces stop the live text with an alert, the recording goes on", async ({
  page,
}) => {
  await openLive(page);
  await publish(page, { state: "live" }, CONVERSATION.slice(0, 2));
  await publish(page, { reason: "engine_failed", state: "unavailable" });

  await expect(page.getByRole("alert")).toHaveText(
    "Live text stopped. The recording continues and the transcript will be made after you stop.",
  );
  await expect(page.getByText("Parfait, merci.")).toHaveCount(0);
  await expect(
    page.getByText("Bonjour à tous, on commence par le point produit ?"),
  ).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Stop", exact: true }),
  ).toBeEnabled();
});

test("after Stop the draft stays greyed under the processing banner", async ({
  page,
}) => {
  await openLive(page);
  await publish(page, { state: "live" }, CONVERSATION);
  await page.getByRole("button", { name: "Stop", exact: true }).click();

  await expect(
    page.getByText(
      "Draft. The final transcript replaces it when processing ends.",
    ),
  ).toBeVisible();
  await expect(
    page.getByRole("heading", { name: "Live transcript" }),
  ).toHaveCount(0);
  await expect(page.getByText("Parfait, merci.")).toBeVisible();
  expect(await callsTo(page, "stop_meeting")).toBe(1);
  expect(
    await page.evaluate(() => window.settingsPages.unhandledCommands),
  ).toEqual([]);
});

test("the live transcript can be turned off in Advanced", async ({ page }) => {
  await page.goto("/tests/fixtures/settings-pages.html?meetings=1");
  await page
    .getByRole("navigation")
    .getByRole("button", { name: "Advanced", exact: true })
    .click();
  const toggle = page.getByRole("checkbox", {
    name: "Live meeting transcript",
  });
  await expect(toggle).toBeChecked();
  await toggle.uncheck({ force: true });
  await expect
    .poll(() => callsTo(page, "change_meeting_live_transcript_setting"))
    .toBe(1);
});

test.describe("live view screenshots", () => {
  test.skip(!outputDir, "Set MURMUR_SCREENSHOT_DIR to capture screenshots");
  test.use({ viewport: DEFAULT_WINDOW });

  for (const theme of ["light", "dark"] as const) {
    const shot = async (page: Page, name: string) => {
      await page.waitForTimeout(400);
      await page.screenshot({ path: `${outputDir}/live-${name}-${theme}.png` });
    };

    test(`live view states (${theme})`, async ({ page }) => {
      await page.emulateMedia({ colorScheme: theme });
      await openLive(page, `&theme=${theme}`);
      await shot(page, "waiting");

      await publish(page, { state: "live" }, CONVERSATION);
      await expect(page.getByText("Parfait, merci.")).toBeVisible();
      await shot(page, "text");

      await publish(page, { state: "catching_up" });
      await expect(page.getByText("Catching up")).toBeVisible();
      await shot(page, "catching-up");

      await publish(page, { reason: "engine_failed", state: "unavailable" });
      await expect(page.getByRole("alert")).toBeVisible();
      await shot(page, "error");

      await page.getByRole("button", { name: "Stop", exact: true }).click();
      await expect(page.getByText(/^Draft\./)).toBeVisible();
      await shot(page, "stopped");

      await openLive(page, `&theme=${theme}&models=missing&live=missing`);
      await expect(
        page.getByRole("button", { name: "Download" }).first(),
      ).toBeVisible();
      await shot(page, "cloud-only");
    });
  }
});
