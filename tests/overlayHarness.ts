import { expect, type Page } from "@playwright/test";

// CARD_MARGIN, OVERLAY_* and ERROR_OVERLAY_* in src-tauri/src/overlay.rs.
const CARD_MARGIN = 16;
export const RECORDING_WINDOW = { width: 158, height: 62 };
export const ERROR_WINDOW = { width: 312, height: 116 };

/** Opens the overlay page with Tauri's event bridge mocked. */
export async function openOverlay(
  page: Page,
  {
    holdSettings = false,
    copyError,
  }: { holdSettings?: boolean; copyError?: string } = {},
) {
  const options = { hold: holdSettings, copyRejection: copyError };
  await page.addInitScript(({ hold, copyRejection }) => {
    const callbacks = new Map<number, (event: unknown) => unknown>();
    const listeners = new Map<number, string>();
    const pendingSettings: Array<(settings: unknown) => void> = [];
    let copyCalls = 0;
    const copyFits: Array<{ width: number; height: number }> = [];
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
      copyCalls: () => copyCalls,
      copyFits: () => copyFits,
    };

    window.__TAURI_INTERNALS__ = {
      invoke: (
        command: string,
        args?: {
          event?: string;
          handler?: number;
          width?: number;
          height?: number;
        },
      ) => {
        if (command === "plugin:event|listen") {
          listeners.set(args?.handler ?? 0, args?.event ?? "");
          return Promise.resolve(args?.handler);
        }
        if (command === "plugin:event|unlisten") return Promise.resolve();
        if (command === "get_app_settings") {
          if (!hold) return Promise.resolve({});
          return new Promise((resolve) => pendingSettings.push(resolve));
        }
        if (command === "copy_notice_again") {
          copyCalls += 1;
          // Tauri rejects a command the ACL denies with a plain string.
          if (copyRejection) return Promise.reject(copyRejection);
          return Promise.resolve(null);
        }
        if (command === "fit_copy_notice") {
          copyFits.push({ width: args?.width ?? 0, height: args?.height ?? 0 });
          return Promise.resolve(null);
        }
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
  }, options);

  await page.goto("/tests/fixtures/recording-overlay-listeners.html");
  await expect
    .poll(() =>
      page.evaluate(() => window.overlayHarness.dispatch("mic-level", [0])),
    )
    .toBe(1);
}

export const dispatch = (page: Page, event: string, payload?: unknown) =>
  page.evaluate(
    ([name, value]) => window.overlayHarness.dispatch(name as string, value),
    [event, payload] as const,
  );

/** Resizes the page the way `fit_copy_notice` resizes the overlay window:
 * the last capsule size the copy pill measured plus CARD_MARGIN around it. */
export async function fitCopyWindow(page: Page) {
  await page.evaluate(() => new Promise(requestAnimationFrame));
  await expect
    .poll(() => page.evaluate(() => window.overlayHarness.copyFits().length))
    .toBeGreaterThan(0);
  const capsule = await page.evaluate(() => {
    const fits = window.overlayHarness.copyFits();
    return fits[fits.length - 1];
  });
  const size = {
    width: Math.ceil(capsule.width) + 2 * CARD_MARGIN,
    height: Math.ceil(capsule.height) + 2 * CARD_MARGIN,
  };
  await page.setViewportSize(size);
  return size;
}

// Drawn sizes include the capsule's pop-in scale until it ends.
export const entranceFinished = (page: Page) =>
  page
    .locator(".ov-capsule")
    .evaluate((capsule) =>
      Promise.all(capsule.getAnimations().map((entry) => entry.finished)),
    );
