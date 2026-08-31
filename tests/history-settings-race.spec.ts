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
  const settledCommitBeforeResponse = await page.evaluate(() =>
    window.historyRace.settledCommitCount(),
  );

  await page.evaluate(
    (entry) => window.historyRace.emit({ action: "added", entry }),
    liveEntry,
  );
  await page.evaluate(
    (entry) => window.historyRace.resolveFirstPage([entry]),
    olderEntry,
  );
  await page.waitForFunction(
    (previousCommit) =>
      window.historyRace.inFlightHistoryRequests() === 0 &&
      window.historyRace.settledCommitCount() > previousCommit,
    settledCommitBeforeResponse,
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

test("loads history after listener registration rejects", async ({ page }) => {
  await page.goto(
    "/tests/fixtures/history-settings-race.html?mode=listen-reject",
  );
  await page.waitForFunction(
    () => window.historyRace.listenRequestCount() === 1,
  );
  await page.waitForFunction(
    () => window.historyRace.historyRequestCount() === 1,
  );

  const fallbackEntry = historyEntry({
    id: 707,
    transcription_text: "History after listener registration rejection",
  });
  await page.evaluate((entry) => {
    window.historyRace.resolveHistoryRequest(0, [entry], false);
  }, fallbackEntry);

  await expect(
    page.getByText("History after listener registration rejection", {
      exact: true,
    }),
  ).toBeVisible();
  await expect
    .poll(() =>
      page.evaluate(() => window.historyRace.inFlightHistoryRequests()),
    )
    .toBe(0);
  await expect
    .poll(() => page.evaluate(() => window.historyRace.unhandledRejections()))
    .toBe(0);
});

test("queues a first-page refresh after a delete rollback during pagination", async ({
  page,
}) => {
  await page.goto(
    "/tests/fixtures/history-settings-race.html?mode=delete-refresh",
  );
  await page.waitForFunction(() => window.historyRace?.ready === true);

  const restoredEntry = historyEntry({
    id: 505,
    transcription_text: "Restored after delete failure",
  });
  await page.evaluate((entry) => {
    window.historyRace.resolveHistoryRequest(0, [entry], true);
  }, restoredEntry);
  await expect(
    page.getByText("Restored after delete failure", { exact: true }),
  ).toBeVisible();
  await page.waitForFunction(
    () => window.historyRace.inFlightHistoryRequests() === 0,
  );

  await page.evaluate(() => window.historyRace.triggerPagination());
  await page.waitForFunction(
    () => window.historyRace.historyRequestCount() === 2,
  );
  await page.evaluate(() => window.historyRace.failNextDelete());
  await page.getByRole("button", { name: "Delete entry" }).click();

  await page.evaluate(() =>
    window.historyRace.resolveHistoryRequest(1, [], false),
  );
  await page.waitForFunction(
    () => window.historyRace.historyRequestCount() === 3,
  );
  await expect
    .poll(() => page.evaluate(() => window.historyRace.historyRequestCursors()))
    .toEqual([null, 505, null]);
  await expect
    .poll(() =>
      page.evaluate(() => window.historyRace.maxInFlightHistoryRequests()),
    )
    .toBe(1);

  await page.evaluate((entry) => {
    window.historyRace.resolveHistoryRequest(2, [entry], false);
  }, restoredEntry);
  await expect(
    page.getByText("Restored after delete failure", { exact: true }),
  ).toBeVisible();
});

test("waits for the history listener before capturing the first page", async ({
  page,
}) => {
  await page.goto(
    "/tests/fixtures/history-settings-race.html?mode=delayed-listen",
  );
  await page.waitForFunction(
    () => window.historyRace.listenRequestCount() === 1,
  );

  const liveEntry = historyEntry({
    id: 606,
    transcription_text: "Persisted before listener registration",
  });
  await page.evaluate(
    (entry) =>
      window.historyRace.emitBeforeListener({ action: "added", entry }),
    liveEntry,
  );
  await page.evaluate(() => window.historyRace.resolveListener());
  await page.waitForFunction(
    () => window.historyRace.historyRequestCount() === 1,
  );
  await page.evaluate(() =>
    window.historyRace.resolveCapturedHistoryRequest(0),
  );

  await expect(
    page.getByText("Persisted before listener registration", { exact: true }),
  ).toBeVisible();
});

test("cleans up a listener that registers after the component unmounts", async ({
  page,
}) => {
  await page.goto(
    "/tests/fixtures/history-settings-race.html?mode=delayed-listen",
  );
  await page.waitForFunction(
    () => window.historyRace.listenRequestCount() === 1,
  );

  await page.evaluate(() => window.historyRace.unmount());
  await page.evaluate(() => window.historyRace.resolveListener());
  await page.waitForFunction(() => window.historyRace.unlistenAttempts() === 1);
  await expect
    .poll(() => page.evaluate(() => window.historyRace.unhandledRejections()))
    .toBe(0);
});
