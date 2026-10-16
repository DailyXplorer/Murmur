import { expect, test } from "@playwright/test";

test("an error that arrives while the overlay loads its settings stays on screen", async ({
  page,
}) => {
  await page.addInitScript(() => {
    const callbacks = new Map<number, (event: unknown) => unknown>();
    const listeners = new Map<number, string>();
    const pendingSettings: Array<(settings: unknown) => void> = [];
    let nextId = 1;

    window.overlayHarness = {
      dispatch: (event, payload) => {
        const ids = [...listeners].filter(([, name]) => name === event);
        ids.forEach(([id]) => callbacks.get(id)?.({ event, id, payload }));
        return ids.length;
      },
      releaseSettings: () => {
        pendingSettings.splice(0).forEach((resolve) => resolve({}));
      },
    };

    window.__TAURI_INTERNALS__ = {
      invoke: (
        command: string,
        args?: { event?: string; handler?: number },
      ) => {
        if (command === "plugin:event|listen") {
          listeners.set(args?.handler ?? 0, args?.event ?? "");
          return Promise.resolve(args?.handler);
        }
        if (command === "plugin:event|unlisten") return Promise.resolve();
        if (command === "get_app_settings") {
          return new Promise((resolve) => pendingSettings.push(resolve));
        }
        if (command === "plugin:os|locale") return Promise.resolve("en-US");
        throw new Error(`Unexpected Tauri command: ${command}`);
      },
      transformCallback: (callback: (event: unknown) => unknown) => {
        const id = nextId++;
        callbacks.set(id, callback);
        return id;
      },
      unregisterCallback: (id: number) => callbacks.delete(id),
    };
    window.__TAURI_EVENT_PLUGIN_INTERNALS__ = {
      unregisterListener: (_event: string, id: number) => {
        listeners.delete(id);
        callbacks.delete(id);
      },
    };
  });

  await page.goto("/tests/fixtures/recording-overlay-listeners.html");
  await expect
    .poll(() =>
      page.evaluate(() => window.overlayHarness.dispatch("mic-level", [0])),
    )
    .toBe(1);

  await page.evaluate(async () => {
    window.overlayHarness.dispatch("show-overlay", "recording");
    window.overlayHarness.dispatch(
      "show-overlay-error",
      "Microphone access is off",
    );
    window.overlayHarness.releaseSettings();
    await new Promise((resolve) => window.setTimeout(resolve, 100));
  });

  await expect(page.getByRole("alert")).toHaveText("Microphone access is off");
  await expect(page.locator(".swave")).toHaveCount(0);
});
