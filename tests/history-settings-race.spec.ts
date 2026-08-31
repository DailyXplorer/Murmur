import { expect, test, type Page } from "@playwright/test";

interface HistoryEntry {
  id: number;
  transcription_text: string;
}

const historyEntry = ({ id, transcription_text }: HistoryEntry) => ({
  file_name: `history-${id}.wav`,
  id,
  saved: false,
  timestamp: 1_700_000_000 + id,
  title: `History ${id}`,
  transcription_text,
});

const openRaceFixture = async (page: Page) => {
  await page.goto("/tests/fixtures/history-settings-race.html");
  await page.waitForFunction(() => window.historyRace?.ready === true);
};

test("keeps an added entry that arrives before a stale first page", async ({
  page,
}) => {
  await openRaceFixture(page);

  const liveEntry = historyEntry({ id: 101, transcription_text: "Live entry" });
  const olderEntry = historyEntry({ id: 1, transcription_text: "Older entry" });

  await page.evaluate(
    (entry) => window.historyRace.emit({ action: "added", entry }),
    liveEntry,
  );
  await page.evaluate(
    (entry) => window.historyRace.resolveFirstPage([entry]),
    olderEntry,
  );

  await expect(page.getByText("Live entry", { exact: true })).toBeVisible();
  await expect(page.getByText("Older entry", { exact: true })).toBeVisible();
});

test("deduplicates an added entry already returned by the first page", async ({
  page,
}) => {
  await openRaceFixture(page);

  const sharedEntry = historyEntry({
    id: 202,
    transcription_text: "Shared entry",
  });

  await page.evaluate(
    (entry) => window.historyRace.resolveFirstPage([entry]),
    sharedEntry,
  );
  await expect(page.getByText("Shared entry", { exact: true })).toBeVisible();
  await page.evaluate(
    (entry) => window.historyRace.emit({ action: "added", entry }),
    sharedEntry,
  );

  await expect(page.getByText("Shared entry", { exact: true })).toHaveCount(1);
});

test("keeps an update that arrives while the first page is in flight", async ({
  page,
}) => {
  await openRaceFixture(page);

  const staleEntry = historyEntry({
    id: 303,
    transcription_text: "Stale transcription",
  });
  const updatedEntry = historyEntry({
    id: 303,
    transcription_text: "Updated transcription",
  });

  await page.evaluate(
    (entry) => window.historyRace.emit({ action: "updated", entry }),
    updatedEntry,
  );
  await page.evaluate(
    (entry) => window.historyRace.resolveFirstPage([entry]),
    staleEntry,
  );

  await expect(
    page.getByText("Updated transcription", { exact: true }),
  ).toBeVisible();
  await expect(
    page.getByText("Stale transcription", { exact: true }),
  ).toHaveCount(0);
});

test("handles an unlisten rejection during cleanup", async ({ page }) => {
  await page.goto(
    "/tests/fixtures/history-settings-race.html?mode=unlisten-reject",
  );
  await page.waitForFunction(() => window.historyRace?.ready === true);

  await page.evaluate(() => window.historyRace.unmount());
  await page.waitForFunction(() => window.historyRace.unlistenAttempts() === 1);
  const unhandledRejections = await page.evaluate(async () => {
    await Promise.resolve();
    await Promise.resolve();
    return window.historyRace.unhandledRejections();
  });

  expect(unhandledRejections).toBe(0);
});
