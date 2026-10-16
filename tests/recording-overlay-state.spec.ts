import { expect, test, type Page } from "@playwright/test";
import { readdirSync, readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

const LOUD = Array(16).fill(0.9);

const localesDir = fileURLToPath(
  new URL("../src/i18n/locales/", import.meta.url),
);
const errorStringsByLocale = readdirSync(localesDir).map((locale) => {
  const { tray } = JSON.parse(
    readFileSync(`${localesDir}${locale}/translation.json`, "utf8"),
  ) as { tray: Record<string, string> };
  const messages = Object.entries(tray)
    .filter(([key]) => key.startsWith("error"))
    .map(([, text]) => text.replace("{{shortcut}}", "⌥ Space"));
  return { locale, messages };
});

// ERROR_OVERLAY_WIDTH × ERROR_OVERLAY_HEIGHT in src-tauri/src/overlay.rs.
const ERROR_WINDOW = { width: 312, height: 100 };

async function openOverlay(page: Page, { holdSettings = false } = {}) {
  await page.addInitScript((hold) => {
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
          if (!hold) return Promise.resolve({});
          return new Promise((resolve) => pendingSettings.push(resolve));
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
  }, holdSettings);

  await page.goto("/tests/fixtures/recording-overlay-listeners.html");
  await expect
    .poll(() =>
      page.evaluate(() => window.overlayHarness.dispatch("mic-level", [0])),
    )
    .toBe(1);
}

const dispatch = (page: Page, event: string, payload?: unknown) =>
  page.evaluate(
    ([name, value]) => window.overlayHarness.dispatch(name as string, value),
    [event, payload] as const,
  );

const centerBarHeight = (page: Page) =>
  page
    .locator(".ov-wave i")
    .nth(6)
    .evaluate((bar) => parseFloat(getComputedStyle(bar).height));

const capsuleWidth = (page: Page) =>
  page
    .locator(".ov-capsule")
    .evaluate((capsule) => parseFloat(getComputedStyle(capsule).width));

test("an error that arrives while the overlay loads its settings stays on screen", async ({
  page,
}) => {
  await openOverlay(page, { holdSettings: true });

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
  await expect(page.getByRole("alert")).toBeVisible();
  await expect(page.locator(".ov-wave")).toHaveCount(0);
});

test("recording shows only a waveform that follows the voice", async ({
  page,
}) => {
  await openOverlay(page);
  await dispatch(page, "show-overlay", "recording");
  await expect(page.locator(".ov-capsule")).toHaveClass(/arming/);

  await dispatch(page, "recording-ready");
  await expect(page.locator(".ov-capsule")).toHaveClass(/live/);
  expect(await centerBarHeight(page)).toBe(4);
  expect(await capsuleWidth(page)).toBe(108);

  await dispatch(page, "mic-level", LOUD);
  await expect.poll(() => centerBarHeight(page)).toBeGreaterThan(20);
  await expect.poll(() => capsuleWidth(page)).toBeGreaterThan(124);

  await expect(page.locator(".ov-wave i")).toHaveCount(13);
  await expect(page.locator(".ov-stage")).toHaveText("");
  await expect(page.locator(".ov-stage button")).toHaveCount(0);
  await expect(page.locator(".ov-stage svg")).toHaveCount(0);
});

test("a hands-free recording looks the same as push-to-talk", async ({
  page,
}) => {
  await openOverlay(page);
  await dispatch(page, "show-overlay", "recording");
  await dispatch(page, "recording-ready");

  expect(await dispatch(page, "overlay-hands-free")).toBe(0);
  expect(await dispatch(page, "recording-limit-warning", 60)).toBe(0);
  await expect(page.locator(".ov-stage")).toHaveText("");
  await expect(page.locator(".ov-capsule")).toHaveClass(/live/);
});

test("transcribing pulses the waveform without any text", async ({ page }) => {
  await openOverlay(page);
  await dispatch(page, "show-overlay", "recording");
  await dispatch(page, "recording-ready");
  await dispatch(page, "mic-level", LOUD);
  await expect.poll(() => centerBarHeight(page)).toBeGreaterThan(20);

  await dispatch(page, "show-overlay", "transcribing");
  await expect(page.locator(".ov-capsule")).toHaveClass(/processing/);
  await expect(page.locator(".ov-stage")).toHaveText("");
  await expect.poll(() => capsuleWidth(page)).toBe(108);

  const heights = new Set<number>();
  await expect
    .poll(async () => {
      heights.add(await centerBarHeight(page));
      return heights.size;
    })
    .toBeGreaterThan(2);
  expect(Math.max(...heights)).toBeLessThanOrEqual(10);
});

test("an error replaces the waveform with the message", async ({ page }) => {
  await openOverlay(page);
  await dispatch(page, "show-overlay", "transcribing");
  await dispatch(page, "show-overlay-error", "Couldn't reach Codex.");

  await expect(page.getByRole("alert")).toHaveText("Couldn't reach Codex.");
  await expect(page.getByRole("alert")).toBeVisible();
  await expect(page.locator(".ov-wave")).toHaveCount(0);
});

for (const { locale, messages } of errorStringsByLocale) {
  test(`every ${locale} error message fits the error window unclipped`, async ({
    page,
  }) => {
    await page.setViewportSize(ERROR_WINDOW);
    await openOverlay(page);
    await page.evaluate(() => document.fonts.ready);

    for (const message of messages) {
      await dispatch(page, "show-overlay-error", message);
      const alert = page.getByRole("alert");
      await expect(alert).toHaveText(message);

      const fit = await alert.evaluate((capsule) => {
        const text = capsule.querySelector("span")!;
        const box = capsule.getBoundingClientRect();
        return {
          clipped: text.scrollHeight > text.clientHeight,
          insideWindow: box.top >= 0 && box.bottom <= window.innerHeight,
        };
      });
      expect(fit, message).toEqual({ clipped: false, insideWindow: true });
    }
  });
}

test("reduced motion shows static bars that ignore the voice", async ({
  page,
}) => {
  await page.emulateMedia({ reducedMotion: "reduce" });
  await openOverlay(page);
  await dispatch(page, "show-overlay", "recording");
  await dispatch(page, "recording-ready");
  await dispatch(page, "mic-level", LOUD);
  await page.waitForTimeout(300);

  expect(await centerBarHeight(page)).toBeCloseTo(12.23, 1);
  expect(await capsuleWidth(page)).toBe(108);

  await dispatch(page, "show-overlay", "transcribing");
  await expect(page.locator(".ov-capsule")).toHaveClass(/processing/);
  await page.waitForTimeout(300);
  expect(await centerBarHeight(page)).toBeCloseTo(12.23, 1);
});
