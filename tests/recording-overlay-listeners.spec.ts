import { expect, test } from "@playwright/test";

const expectedListenerCounts = {
  "show-overlay": 2,
  "hide-overlay": 2,
  "recording-ready": 2,
  "mic-level": 2,
};

const countEvents = (events: string[]) =>
  Object.fromEntries(
    Object.keys(expectedListenerCounts).map((event) => [
      event,
      events.filter((candidate) => candidate === event).length,
    ]),
  );

test("cleans every settled overlay listener exactly once", async ({ page }) => {
  await page.addInitScript(() => {
    type PendingListener = {
      event: string;
      id: number;
      reject: (reason?: unknown) => void;
      resolve: (value: number) => void;
    };

    const pendingListeners: PendingListener[] = [];
    const successfulListenerIds: number[] = [];
    const unlistenCounts = new Map<number, number>();
    const callbacks = new Map<number, (event: unknown) => unknown>();
    const registeredListeners = new Map<number, string>();
    const listenerAttempts: string[] = [];
    let nextListenerId = 1;
    let rejectedListener = false;
    const runCallback = (id: number, event: unknown) => {
      const callback = callbacks.get(id);
      if (!callback) return false;

      callback(event);
      return true;
    };

    Object.assign(window, {
      recordingOverlayListenerHarness: {
        pendingCount: () => pendingListeners.length,
        resolveNext: () => {
          const listener = pendingListeners.shift();
          if (!listener) return false;

          if (!rejectedListener && listener.event === "hide-overlay") {
            rejectedListener = true;
            listener.reject(new Error("listener registration failed"));
          } else {
            successfulListenerIds.push(listener.id);
            registeredListeners.set(listener.id, listener.event);
            listener.resolve(listener.id);
          }

          return true;
        },
        settleUntilIdle: async () => {
          let idlePasses = 0;

          while (idlePasses < 2) {
            while (pendingListeners.length > 0) {
              window.recordingOverlayListenerHarness.resolveNext();
            }

            await new Promise((resolve) => window.setTimeout(resolve, 0));
            idlePasses = pendingListeners.length === 0 ? idlePasses + 1 : 0;
          }
        },
        dispatch: (event: string, payload?: unknown) => {
          const listeners = Array.from(registeredListeners.entries()).filter(
            ([, registeredEvent]) => registeredEvent === event,
          );

          const delivered = listeners.map(([id]) =>
            runCallback(id, { event, id, payload }),
          );

          return delivered.filter(Boolean).length;
        },
        dispatchById: (id: number, event: string, payload?: unknown) => {
          return runCallback(id, { event, id, payload });
        },
        summary: () => ({
          rejectedListener,
          successfulListenerIds,
          unlistenCounts: Array.from(unlistenCounts.entries()),
          listenerAttempts,
          registeredListeners: Array.from(registeredListeners.entries()),
        }),
      },
    });

    window.__TAURI_INTERNALS__ = {
      invoke: (
        command: string,
        args?: { event?: string; eventId?: number; handler?: number },
      ) => {
        if (command === "plugin:event|listen") {
          const id = args?.handler;
          if (typeof id !== "number") {
            throw new Error("Tauri listener callback id is missing");
          }

          listenerAttempts.push(args?.event ?? "");
          return new Promise<number>((resolve, reject) => {
            pendingListeners.push({
              event: args?.event ?? "",
              id,
              reject,
              resolve,
            });
          });
        }

        if (command === "get_app_settings") return Promise.resolve({});

        if (command === "plugin:event|unlisten") return Promise.resolve();

        throw new Error(`Unexpected Tauri command: ${command}`);
      },
      transformCallback: (callback: (event: unknown) => unknown) => {
        const id = nextListenerId++;
        callbacks.set(id, callback);
        return id;
      },
      runCallback,
      unregisterCallback: (id: number) => callbacks.delete(id),
    };

    window.__TAURI_EVENT_PLUGIN_INTERNALS__ = {
      unregisterListener: (_event: string, id: number) => {
        unlistenCounts.set(id, (unlistenCounts.get(id) ?? 0) + 1);
        registeredListeners.delete(id);
        callbacks.delete(id);
      },
    };
  });

  await page.goto("/tests/fixtures/recording-overlay-listeners.html");
  await expect
    .poll(() =>
      page.evaluate(() =>
        window.recordingOverlayListenerHarness.pendingCount(),
      ),
    )
    .toBe(8);

  const initialSummary = await page.evaluate(() =>
    window.recordingOverlayListenerHarness.summary(),
  );
  expect(countEvents(initialSummary.listenerAttempts)).toEqual(
    expectedListenerCounts,
  );

  await page.evaluate(() =>
    window.recordingOverlayListenerHarness.resolveNext(),
  );
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          window.recordingOverlayListenerHarness.summary().unlistenCounts
            .length,
      ),
    )
    .toBe(1);

  await page.evaluate(async () => {
    await window.recordingOverlayListenerHarness.settleUntilIdle();
  });

  const mountedSummary = await page.evaluate(() =>
    window.recordingOverlayListenerHarness.summary(),
  );
  expect(mountedSummary.rejectedListener).toBe(true);
  expect(
    mountedSummary.registeredListeners.map(([, event]) => event).sort(),
  ).toEqual(Object.keys(expectedListenerCounts).sort());

  const deliveredEvents = await page.evaluate(() => ({
    show: window.recordingOverlayListenerHarness.dispatch(
      "show-overlay",
      "recording",
    ),
    ready: window.recordingOverlayListenerHarness.dispatch("recording-ready"),
    level: window.recordingOverlayListenerHarness.dispatch("mic-level", [4]),
  }));
  expect(deliveredEvents).toEqual({ show: 1, ready: 1, level: 1 });
  await expect(page.locator(".ov-stage")).toBeVisible();
  await expect(page.locator(".sdot")).toHaveClass(/ready/);
  await expect(page.locator(".swave i").first()).toHaveCSS("height", "18px");

  const hidden = await page.evaluate(() =>
    window.recordingOverlayListenerHarness.dispatch("hide-overlay"),
  );
  expect(hidden).toBe(1);
  await expect(page.locator(".ov-stage")).toHaveCount(0);

  await page.evaluate(() => window.unmountRecordingOverlay());

  const summary = await page.evaluate(() =>
    window.recordingOverlayListenerHarness.summary(),
  );
  const unlistenIds = summary.unlistenCounts.map(([id]) => id);
  const postCleanupDispatch = await page.evaluate(() => {
    const listenerIds =
      window.recordingOverlayListenerHarness.summary().successfulListenerIds;

    return {
      show: window.recordingOverlayListenerHarness.dispatch(
        "show-overlay",
        "recording",
      ),
      hide: window.recordingOverlayListenerHarness.dispatch("hide-overlay"),
      ready: window.recordingOverlayListenerHarness.dispatch("recording-ready"),
      level: window.recordingOverlayListenerHarness.dispatch("mic-level", [4]),
      callbacks: listenerIds.map((id) =>
        window.recordingOverlayListenerHarness.dispatchById(
          id,
          "recording-ready",
        ),
      ),
    };
  });

  expect(summary.rejectedListener).toBe(true);
  expect(unlistenIds).toHaveLength(summary.successfulListenerIds.length);
  expect(new Set(unlistenIds)).toEqual(new Set(summary.successfulListenerIds));
  expect(summary.unlistenCounts.every(([, count]) => count === 1)).toBe(true);
  expect(summary.registeredListeners).toEqual([]);
  expect(postCleanupDispatch).toEqual({
    show: 0,
    hide: 0,
    ready: 0,
    level: 0,
    callbacks: summary.successfulListenerIds.map(() => false),
  });
});
