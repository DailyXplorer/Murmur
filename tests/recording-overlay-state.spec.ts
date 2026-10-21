import { expect, test, type Page } from "@playwright/test";
import { readdirSync, readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import {
  COPY_WINDOW,
  dispatch,
  entranceFinished,
  ERROR_WINDOW,
  openOverlay,
  RECORDING_WINDOW,
} from "./overlayHarness";

const LOUD = Array(16).fill(0.9);

const localesDir = fileURLToPath(
  new URL("../src/i18n/locales/", import.meta.url),
);
const trayStringsByLocale = readdirSync(localesDir).map((locale) => {
  const { tray } = JSON.parse(
    readFileSync(`${localesDir}${locale}/translation.json`, "utf8"),
  ) as { tray: Record<string, string> };
  return { locale, tray };
});
const errorStringsByLocale = trayStringsByLocale.map(({ locale, tray }) => {
  const messages = Object.entries(tray)
    .filter(([key]) => key.startsWith("error"))
    .map(([, text]) => text.replace("{{shortcut}}", "⌥ Space"));
  return { locale, messages };
});

const COPY_LABELS = { copy: "Copy", copied: "Copied" };

const centerBarHeight = (page: Page) =>
  page
    .locator(".ov-wave i")
    .nth(6)
    .evaluate((bar) => bar.getBoundingClientRect().height);

// Feeds a speech-like stream at the host's ~30 Hz (words of ~5 syllables a
// second, short pauses, per-event noise; fixed seed) and samples the capsule
// and bars on every animation frame.
const sampleWhileSpeaking = (page: Page, seconds: number) =>
  page.evaluate(async (durationMs) => {
    const capsule = document.querySelector<HTMLElement>(".ov-capsule")!;
    const bars = [...document.querySelectorAll<HTMLElement>(".ov-wave i")];
    let seed = 7;
    const noise = () => {
      seed = (seed * 16807) % 2147483647;
      return seed / 2147483647 - 0.5;
    };
    const start = performance.now();
    const timer = window.setInterval(() => {
      const t = (performance.now() - start) / 1000;
      const inWord = t % 0.85 < 0.6;
      const syllable = Math.abs(Math.sin(t * Math.PI * 5));
      const levels = Array.from({ length: 16 }, (_, band) => {
        const voice = inWord ? 0.2 + 0.6 * syllable : 0.03;
        const level = voice * (band < 6 ? 1 : 0.6) + noise() * 0.3;
        return Math.min(1, Math.max(0, level));
      });
      window.overlayHarness.dispatch("mic-level", levels);
    }, 33);

    const frames: Array<{
      left: number;
      right: number;
      height: number;
      barLayoutHeights: number[];
      centerBarDrawnHeight: number;
    }> = [];
    await new Promise<void>((resolve) => {
      requestAnimationFrame(function sample() {
        const box = capsule.getBoundingClientRect();
        frames.push({
          left: box.left,
          right: box.right,
          height: box.height,
          barLayoutHeights: bars.map((bar) =>
            parseFloat(getComputedStyle(bar).height),
          ),
          centerBarDrawnHeight: bars[6].getBoundingClientRect().height,
        });
        if (performance.now() - start < durationMs) {
          requestAnimationFrame(sample);
        } else {
          resolve();
        }
      });
    });
    window.clearInterval(timer);
    return frames;
  }, seconds * 1000);

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
  await entranceFinished(page);
  expect(await centerBarHeight(page)).toBeCloseTo(4, 1);
  expect(await capsuleWidth(page)).toBe(108);

  await dispatch(page, "mic-level", LOUD);
  await expect.poll(() => centerBarHeight(page)).toBeGreaterThan(16);
  await expect.poll(() => capsuleWidth(page)).toBeGreaterThanOrEqual(120);

  await expect(page.locator(".ov-wave i")).toHaveCount(13);
  await expect(page.locator(".ov-stage")).toHaveText("");
  await expect(page.locator(".ov-stage button")).toHaveCount(0);
  await expect(page.locator(".ov-stage svg")).toHaveCount(0);
});

test("the capsule holds still while the voice wobbles", async ({ page }) => {
  await page.setViewportSize(RECORDING_WINDOW);
  await openOverlay(page);
  await dispatch(page, "show-overlay", "recording");
  await dispatch(page, "recording-ready");
  await expect(page.locator(".ov-capsule")).toHaveClass(/live/);
  await entranceFinished(page);

  const seconds = 3;
  const frames = await sampleWhileSpeaking(page, seconds);
  const lefts = frames.map((frame) => frame.left);
  const steps = lefts.slice(1).map((left, i) => left - lefts[i]);
  const moves = steps.filter((step) => step !== 0);
  const reversals = moves
    .slice(1)
    .filter((step, i) => Math.sign(step) !== Math.sign(moves[i])).length;
  const drawn = frames.map((frame) => frame.centerBarDrawnHeight);
  const relayouts = frames
    .slice(1)
    .filter(
      (frame, i) =>
        frame.barLayoutHeights.join() !== frames[i].barLayoutHeights.join(),
    ).length;
  test.info().annotations.push({
    type: "jitter",
    description: JSON.stringify({
      frames: frames.length,
      edgeMoves: moves.length,
      reversals,
      fractionalEdges: lefts.filter((left) => !Number.isInteger(left)).length,
      barRelayouts: relayouts,
    }),
  });

  expect(new Set(frames.map((frame) => frame.height))).toEqual(new Set([30]));
  expect(lefts.every(Number.isInteger)).toBe(true);
  expect(
    frames.every(
      (frame) => frame.left + frame.right === RECORDING_WINDOW.width,
    ),
  ).toBe(true);
  expect(moves.length).toBeLessThanOrEqual(4 * seconds);
  expect(reversals).toBeLessThanOrEqual(3);
  expect(Math.max(...drawn) - Math.min(...drawn)).toBeGreaterThan(6);
  expect(relayouts).toBe(0);
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
  await expect.poll(() => centerBarHeight(page)).toBeGreaterThan(16);

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

test("without a text field the pill is one Copy button that confirms the copy", async ({
  page,
}) => {
  await openOverlay(page);
  await dispatch(page, "show-overlay", "transcribing");
  await dispatch(page, "show-overlay-copy", COPY_LABELS);

  const button = page.getByRole("button", { name: "Copy" });
  await expect(button).toBeVisible();
  await expect(page.locator(".ov-stage button")).toHaveCount(1);
  await expect(page.locator(".ov-stage")).toHaveText("Copy");
  await expect(page.locator(".ov-wave")).toHaveCount(0);

  await button.click();
  await expect(page.getByRole("button", { name: "Copied" })).toBeVisible();
  await expect(page.locator(".ov-stage")).toHaveText("Copied");
  expect(await page.evaluate(() => window.overlayHarness.copyCalls())).toBe(1);
});

test("the copy pill hides on request and gives way to a new recording", async ({
  page,
}) => {
  await openOverlay(page);
  await dispatch(page, "show-overlay-copy", COPY_LABELS);
  await expect(page.getByRole("button", { name: "Copy" })).toBeVisible();
  await dispatch(page, "hide-overlay");
  await expect(page.locator(".ov-stage")).toHaveCount(0);

  await dispatch(page, "show-overlay-copy", COPY_LABELS);
  await expect(page.getByRole("button", { name: "Copy" })).toBeVisible();
  await dispatch(page, "show-overlay", "recording");
  await expect(page.locator(".ov-stage button")).toHaveCount(0);
  await expect(page.locator(".ov-wave")).toBeVisible();
});

for (const { locale, tray } of trayStringsByLocale) {
  test(`the ${locale} copy pill fits the copy window unclipped`, async ({
    page,
  }) => {
    await page.setViewportSize(COPY_WINDOW);
    await openOverlay(page);
    await page.evaluate(() => document.fonts.ready);
    const labels = { copy: tray.overlayCopy, copied: tray.overlayCopied };
    expect(labels.copy && labels.copied, "both labels exist").toBeTruthy();

    await dispatch(page, "show-overlay-copy", labels);
    for (const label of [labels.copy, labels.copied]) {
      const button = page.getByRole("button", { name: label, exact: true });
      await expect(button).toBeVisible();
      await entranceFinished(page);
      const fit = await button.evaluate((capsule) => {
        const box = capsule.getBoundingClientRect();
        const text = capsule.querySelector("span")!;
        return {
          clipped:
            capsule.scrollWidth > capsule.clientWidth ||
            text.getBoundingClientRect().right > box.right ||
            text.getBoundingClientRect().left < box.left,
          insideWindow:
            box.left >= 0 &&
            box.right <= window.innerWidth &&
            box.top >= 0 &&
            box.bottom <= window.innerHeight,
          height: box.height,
        };
      });
      expect(fit, label).toEqual({
        clipped: false,
        insideWindow: true,
        height: 30,
      });
      if (label === labels.copy) await button.click();
    }
  });
}

test("reduced motion shows the copy pill without any animation", async ({
  page,
}) => {
  await page.emulateMedia({ reducedMotion: "reduce" });
  await openOverlay(page);
  await dispatch(page, "show-overlay-copy", COPY_LABELS);
  const button = page.getByRole("button", { name: "Copy" });
  await expect(button).toBeVisible();
  expect(await page.evaluate(() => document.getAnimations().length)).toBe(0);

  await button.click();
  await expect(page.getByRole("button", { name: "Copied" })).toBeVisible();
  expect(await page.evaluate(() => document.getAnimations().length)).toBe(0);
});

test("reduced motion shows static bars that ignore the voice", async ({
  page,
}) => {
  await page.emulateMedia({ reducedMotion: "reduce" });
  await openOverlay(page);
  await dispatch(page, "show-overlay", "recording");
  await dispatch(page, "recording-ready");
  await dispatch(page, "mic-level", LOUD);
  await page.waitForTimeout(300);

  expect(await centerBarHeight(page)).toBeCloseTo(10.4, 1);
  expect(await capsuleWidth(page)).toBe(108);

  await dispatch(page, "show-overlay", "transcribing");
  await expect(page.locator(".ov-capsule")).toHaveClass(/processing/);
  await page.waitForTimeout(300);
  expect(await centerBarHeight(page)).toBeCloseTo(10.4, 1);
});

test("the meeting pill ticks without moving, turns amber near the cap and gives way to dictation", async ({
  page,
}) => {
  const now = Date.parse("2026-10-17T10:00:00Z");
  const cap = 3 * 3600 * 1000;
  await page.clock.install({ time: now - 1_000 });
  await page.clock.pauseAt(now);
  await page.setViewportSize(RECORDING_WINDOW);
  await openOverlay(page);
  const pill = page.locator(".ov-meeting");
  const startedAt = now - 65_000;

  await dispatch(page, "show-overlay-meeting", {
    started_at_ms: startedAt,
    stops_at_ms: startedAt + cap,
  });
  await expect(pill).toHaveText("01:05");
  await entranceFinished(page);
  const edges = () =>
    pill.evaluate((capsule) => {
      const box = capsule.getBoundingClientRect();
      return [box.left, box.right, box.top, box.bottom];
    });
  const before = await edges();
  await page.clock.runFor(2_000);
  await expect(pill).toHaveText("01:07");
  expect(await edges()).toEqual(before);
  await expect(pill).not.toHaveClass(/warning/);

  // A dictation takes the overlay over; the backend sends the pill back after.
  await dispatch(page, "show-overlay", "recording");
  await expect(pill).toHaveCount(0);
  await expect(page.locator(".ov-wave")).toBeVisible();

  const nearCap = now + 2_000 - (cap - 4 * 60 * 1000);
  await dispatch(page, "show-overlay-meeting", {
    started_at_ms: nearCap,
    stops_at_ms: nearCap + cap,
  });
  await expect(pill).toHaveText("2:56:00");
  await expect(pill).toHaveClass(/warning/);
});

test("after a sleep past the cap the pill reads the cap until the meeting stops", async ({
  page,
}) => {
  const now = Date.parse("2026-10-17T10:00:00Z");
  const cap = 3 * 3600 * 1000;
  await page.clock.install({ time: now - 1_000 });
  await page.clock.pauseAt(now);
  await page.setViewportSize(RECORDING_WINDOW);
  await openOverlay(page);
  const startedAt = now - cap - 20 * 60 * 1000;

  await dispatch(page, "show-overlay-meeting", {
    started_at_ms: startedAt,
    stops_at_ms: startedAt + cap,
  });
  await expect(page.locator(".ov-meeting")).toHaveText("3:00:00");
});
