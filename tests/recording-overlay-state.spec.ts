import { expect, test, type Page } from "@playwright/test";
import { readdirSync, readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import {
  dispatch,
  entranceFinished,
  ERROR_WINDOW,
  fitCopyWindow,
  MEETING_WINDOW,
  openCallPrompt,
  openOverlay,
  ORB_WINDOW,
  widthsAfter,
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

const COPY_LABELS = {
  copy: "Copy",
  copied: "Copied",
  note: "There was no text field to paste into.",
};

// The drawn size of the orb's ring, which the voice pushes out.
const ringSize = (page: Page) =>
  page.locator(".ov-orb-ring").evaluate((ring: SVGGraphicsElement) => {
    const box = ring.getBBox();
    return Math.max(box.width, box.height);
  });

// Feeds a speech-like stream at the host's ~30 Hz (words of ~5 syllables a
// second, short pauses, per-event noise; fixed seed) and samples the capsule
// and the ring on every animation frame.
const sampleWhileSpeaking = (page: Page, seconds: number) =>
  page.evaluate(async (durationMs) => {
    const capsule = document.querySelector<HTMLElement>(".ov-capsule")!;
    const ring = document.querySelector<SVGGraphicsElement>(".ov-orb-ring")!;
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
      width: number;
      height: number;
      ring: number;
    }> = [];
    await new Promise<void>((resolve) => {
      requestAnimationFrame(function sample() {
        const box = capsule.getBoundingClientRect();
        const drawn = ring.getBBox();
        frames.push({
          left: box.left,
          right: box.right,
          width: box.width,
          height: box.height,
          ring: Math.max(drawn.width, drawn.height),
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
  await expect(page.locator(".ov-orb")).toHaveCount(0);
});

test("recording shows only the orb, whose ring follows the voice", async ({
  page,
}) => {
  await openOverlay(page);
  await dispatch(page, "show-overlay", "recording");
  await expect(page.locator(".ov-capsule")).toHaveClass(/arming/);

  await dispatch(page, "recording-ready");
  await expect(page.locator(".ov-capsule")).toHaveClass(/live/);
  await entranceFinished(page);
  expect(await ringSize(page), "at rest").toBeLessThan(18.5);
  expect(await capsuleWidth(page)).toBe(34);

  await dispatch(page, "mic-level", LOUD);
  await expect.poll(() => ringSize(page)).toBeGreaterThan(21);
  expect(await capsuleWidth(page), "the orb never grows").toBe(34);

  await expect(page.locator(".ov-orb")).toHaveCount(1);
  await expect(page.locator(".ov-stage")).toHaveText("");
  await expect(page.locator(".ov-stage button")).toHaveCount(0);
});

test("the capsule holds still while the voice wobbles", async ({ page }) => {
  await page.setViewportSize(ORB_WINDOW);
  await openOverlay(page);
  await dispatch(page, "show-overlay", "recording");
  await dispatch(page, "recording-ready");
  await expect(page.locator(".ov-capsule")).toHaveClass(/live/);
  await entranceFinished(page);

  const frames = await sampleWhileSpeaking(page, 3);
  const rings = frames.map((frame) => frame.ring);

  expect(
    new Set(frames.map((frame) => `${frame.width}x${frame.height}`)),
  ).toEqual(new Set(["34x34"]));
  expect(frames.every((frame) => Number.isInteger(frame.left))).toBe(true);
  expect(
    frames.every((frame) => frame.left + frame.right === ORB_WINDOW.width),
  ).toBe(true);
  expect(Math.max(...rings) - Math.min(...rings)).toBeGreaterThan(3);
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

test("transcribing runs an arc round the orb without any text", async ({
  page,
}) => {
  await openOverlay(page);
  await dispatch(page, "show-overlay", "recording");
  await dispatch(page, "recording-ready");
  await dispatch(page, "mic-level", LOUD);
  await expect.poll(() => ringSize(page)).toBeGreaterThan(21);

  await dispatch(page, "show-overlay", "transcribing");
  await expect(page.locator(".ov-capsule")).toHaveClass(/processing/);
  await expect(page.locator(".ov-stage")).toHaveText("");
  expect(await capsuleWidth(page)).toBe(34);
  expect(await ringSize(page), "the ring is still again").toBeLessThan(17.5);

  const arc = page.locator(".ov-orb-arc");
  await expect(arc).toHaveCount(1);
  const turns = new Set<string>();
  await expect
    .poll(async () => {
      turns.add(await arc.evaluate((a) => getComputedStyle(a).transform));
      return turns.size;
    })
    .toBeGreaterThan(2);
});

test("an error replaces the orb with the message", async ({ page }) => {
  await openOverlay(page);
  await dispatch(page, "show-overlay", "transcribing");
  await dispatch(page, "show-overlay-error", "Couldn't reach Codex.");

  await expect(page.getByRole("alert")).toHaveText("Couldn't reach Codex.");
  await expect(page.getByRole("alert")).toBeVisible();
  await expect(page.locator(".ov-orb")).toHaveCount(0);
});

test("an error opens from the orb to its message", async ({ page }) => {
  await page.setViewportSize(ERROR_WINDOW);
  await openOverlay(page);
  await page.evaluate(() => document.fonts.ready);
  await dispatch(page, "show-overlay", "transcribing");

  const widths = await widthsAfter(
    page,
    "show-overlay-error",
    "Transcription failed. Retry from History.",
    ".ov-error",
  );
  expect(widths[0], "width starts at the orb").toBeLessThanOrEqual(40);
  expect(widths[widths.length - 1], "width ends fitted").toBeGreaterThan(200);
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
        const text = capsule.querySelector(".ov-error-text")!;
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

test("without a text field the pill says so and is one Copy button that confirms the copy", async ({
  page,
}) => {
  await openOverlay(page);
  await dispatch(page, "show-overlay", "transcribing");
  await dispatch(page, "show-overlay-copy", COPY_LABELS);

  const button = page.getByRole("button", { name: "Copy", exact: true });
  await expect(button).toBeVisible();
  await expect(button).toHaveAccessibleDescription(COPY_LABELS.note);
  await expect(page.locator(".ov-stage button")).toHaveCount(1);
  await expect(button.getByText(COPY_LABELS.note)).toBeVisible();
  await expect(page.locator(".ov-orb")).toHaveCount(0);
  const copyCalls = () =>
    page.evaluate(() => window.overlayHarness.copyCalls());
  expect(await copyCalls(), "the pill asks for no copy on its own").toBe(0);

  await button.click();
  const copied = page.getByRole("button", { name: "Copied", exact: true });
  await expect(copied).toBeVisible();
  await expect(copied).toHaveAccessibleDescription(COPY_LABELS.note);
  await expect(copied.getByText(COPY_LABELS.note)).toBeVisible();
  expect(await copyCalls()).toBe(1);
});

test("the copy pill opens from the orb to fit its one-line note", async ({
  page,
}) => {
  await openOverlay(page);
  await page.evaluate(() => document.fonts.ready);
  const sizes = await page.evaluate(
    (labels) =>
      new Promise<Array<[number, number]>>((resolve) => {
        window.overlayHarness.dispatch("show-overlay-copy", labels);
        const seen: Array<[number, number]> = [];
        const start = performance.now();
        requestAnimationFrame(function sample(now) {
          const pill = document.querySelector<HTMLElement>(".ov-copy");
          if (pill) seen.push([pill.offsetWidth, pill.offsetHeight]);
          if (now - start < 900) requestAnimationFrame(sample);
          else resolve(seen);
        });
      }),
    COPY_LABELS,
  );

  expect(sizes.length, "the pill showed").toBeGreaterThan(10);
  const widths = sizes.map(([width]) => width);
  expect(widths[0], "width starts at the orb").toBeLessThanOrEqual(40);
  const final = widths[widths.length - 1];
  expect(final, "width ends fitted to the note").toBeGreaterThan(180);
  const steps = widths.slice(1).map((width, i) => width - widths[i]);
  expect(Math.min(...steps), "width never shrinks").toBeGreaterThanOrEqual(0);
  expect(Math.max(...steps), "width grows over frames").toBeLessThan(
    (final - widths[0]) / 2,
  );
  expect(
    new Set(sizes.map(([, height]) => height)),
    "a one-line note keeps the capsule height throughout",
  ).toEqual(new Set([34]));
});

test("a refused copy leaves the pill on Copy and says why in the log", async ({
  page,
}) => {
  const warnings: string[] = [];
  page.on("console", (message) => {
    if (message.type() === "warning") warnings.push(message.text());
  });
  await openOverlay(page, {
    copyError: "copy_notice_again not allowed on window recording_overlay",
  });
  await dispatch(page, "show-overlay-copy", COPY_LABELS);

  await page.getByRole("button", { name: "Copy" }).click();
  await expect
    .poll(() => warnings.join("\n"))
    .toContain("copy_notice_again not allowed");
  await expect(
    page.getByRole("button", { name: "Copy", exact: true }),
  ).toBeVisible();
});

// The backend opens every copy notice at a default size, so a notice that
// replaces a showing one must ask for its fit again.
test("a copy notice that replaces a showing one fits its window again", async ({
  page,
}) => {
  await openOverlay(page);
  await dispatch(page, "show-overlay-copy", COPY_LABELS);
  const first = await fitCopyWindow(page);
  const fits = () =>
    page.evaluate(() => window.overlayHarness.copyFits().length);
  const before = await fits();

  await dispatch(page, "show-overlay-copy", COPY_LABELS);
  await expect.poll(fits).toBeGreaterThan(before);
  expect(await fitCopyWindow(page)).toEqual(first);
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
  await expect(page.locator(".ov-orb")).toBeVisible();
});

// A system font standing in for Instrument Sans, should it fail to load, is
// wider; this face and spacing are wider still, so the pill must size itself
// from what it measures, not from widths that suit one font.
const WIDE_FONT = `.ov-stage {
  font-family: Verdana, "DejaVu Sans", sans-serif !important;
  letter-spacing: 0.08em;
}`;

// The widest copy pill keeps its window, margins included, well inside a
// 1280-px-wide screen.
const WIDEST_PILL = 560;

for (const { locale, tray } of trayStringsByLocale) {
  for (const font of ["app font", "wide font"] as const) {
    test(`the ${locale} copy pill fits its note and action in its window with the ${font}`, async ({
      page,
    }) => {
      await openOverlay(page);
      if (font === "wide font") await page.addStyleTag({ content: WIDE_FONT });
      await page.evaluate(() => document.fonts.ready);
      const labels = {
        copy: tray.overlayCopy,
        copied: tray.overlayCopied,
        note: tray.overlayNotPasted,
      };
      expect(
        labels.copy && labels.copied && labels.note,
        "all three labels exist",
      ).toBeTruthy();
      if (font === "app font") {
        expect(
          await page.evaluate(() =>
            [...document.fonts].some(
              (face) =>
                face.family.replace(/"/g, "") === "Instrument Sans Variable" &&
                face.status === "loaded",
            ),
          ),
          "Instrument Sans is loaded",
        ).toBe(true);
      }

      await dispatch(page, "show-overlay-copy", labels);
      const windows: Array<{ width: number; height: number }> = [];
      for (const label of [labels.copy, labels.copied]) {
        const button = page.getByRole("button", { name: label, exact: true });
        await expect(button).toBeVisible();
        await entranceFinished(page);
        windows.push(await fitCopyWindow(page));
        const fit = await button.evaluate(
          (capsule: HTMLElement, texts) => {
            const box = capsule.getBoundingClientRect();
            const leaves = capsule.querySelectorAll<HTMLElement>(
              "span:not(:has(span))",
            );
            const spans = [...leaves].filter((span) =>
              texts.includes(span.textContent ?? ""),
            );
            const rects = spans.map((span) => span.getBoundingClientRect());
            const note = spans[0];
            const lineHeight = parseFloat(getComputedStyle(note).lineHeight);
            const text = document.createRange();
            text.selectNodeContents(note);
            const lines = [...text.getClientRects()];
            const action = capsule.querySelector(".ov-copy-action")!;
            const actionBox = action.getBoundingClientRect();
            const glyphBox = capsule
              .querySelector(".ov-glyph")!
              .getBoundingClientRect();
            const glyphInset = glyphBox.left - box.left;
            const linesLeft = Math.min(...lines.map((line) => line.left));
            const linesRight = Math.max(...lines.map((line) => line.right));
            const gapToAction = actionBox.left - linesRight;
            const noteInset = linesLeft - box.left;
            const actionInset = box.right - actionBox.right;
            const words = note.firstChild as Text;
            const glyph = document.createRange();
            const lineTexts: string[] = [];
            let lineStart = 0;
            let lineTop: number | null = null;
            for (let i = 0; i < words.length; i++) {
              glyph.setStart(words, i);
              glyph.setEnd(words, i + 1);
              const [rect] = glyph.getClientRects();
              if (!rect || !words.data[i].trim()) continue;
              if (lineTop !== null && rect.top > lineTop + lineHeight / 2) {
                lineTexts.push(words.data.slice(lineStart, i).trim());
                lineStart = i;
              }
              lineTop = rect.top;
            }
            return {
              pillWidth: capsule.offsetWidth,
              shown: spans.map((span) => span.textContent),
              clipped:
                capsule.scrollWidth > capsule.clientWidth ||
                capsule.scrollHeight > capsule.clientHeight ||
                rects.some(
                  (rect) =>
                    rect.right > box.right ||
                    rect.left < box.left ||
                    rect.top < box.top ||
                    rect.bottom > box.bottom,
                ),
              insideWindow:
                box.left >= 0 &&
                box.right <= window.innerWidth &&
                box.top >= 0 &&
                box.bottom <= window.innerHeight,
              noteLines: Math.round(rects[0].height / lineHeight),
              gapToAction,
              glyphInset,
              noteInset,
              actionInset,
              linesEndingInHyphen: lineTexts.filter((line) =>
                /[-\u{2010}\u{2011}]$/u.test(line),
              ),
            };
          },
          [labels.note, label],
        );
        test.info().annotations.push({
          type: "note lines",
          description: `${label}: ${fit.noteLines}`,
        });
        expect(fit.pillWidth, `${label}: pill width`).toBeLessThanOrEqual(
          WIDEST_PILL,
        );
        expect(
          fit.noteLines,
          `${label}: the note takes two lines at most`,
        ).toBeLessThanOrEqual(2);
        expect(
          fit.gapToAction,
          `${label}: note ends by the action`,
        ).toBeGreaterThanOrEqual(0);
        expect(
          fit.gapToAction,
          `${label}: note ends by the action`,
        ).toBeLessThan(12);
        expect(
          fit.glyphInset,
          `${label}: the orb clears the rounded edge`,
        ).toBeGreaterThanOrEqual(8);
        expect(
          fit.glyphInset,
          `${label}: the orb sits in the rounded end`,
        ).toBeLessThanOrEqual(12);
        expect(
          fit.noteInset,
          `${label}: the note starts after the orb`,
        ).toBeGreaterThan(fit.glyphInset + 15);
        expect(
          fit.actionInset,
          `${label}: the action sits in the rounded end`,
        ).toBeGreaterThanOrEqual(4);
        expect(
          fit.actionInset,
          `${label}: the action sits in the rounded end`,
        ).toBeLessThanOrEqual(7);
        expect(fit, label).toEqual({
          pillWidth: fit.pillWidth,
          shown: [labels.note, label],
          clipped: false,
          insideWindow: true,
          noteLines: fit.noteLines,
          gapToAction: fit.gapToAction,
          glyphInset: fit.glyphInset,
          noteInset: fit.noteInset,
          actionInset: fit.actionInset,
          linesEndingInHyphen: [],
        });
        if (label === labels.copy) await button.click();
      }
      expect(windows[1], "keeps its size when it confirms").toEqual(windows[0]);
    });
  }
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

test("reduced motion shows a still orb that ignores the voice", async ({
  page,
}) => {
  await page.emulateMedia({ reducedMotion: "reduce" });
  await openOverlay(page);
  await dispatch(page, "show-overlay", "recording");
  await dispatch(page, "recording-ready");
  await dispatch(page, "mic-level", LOUD);
  const ring = page.locator(".ov-orb-ring");
  const still = await ring.getAttribute("d");
  await page.waitForTimeout(300);

  expect(await ring.getAttribute("d")).toBe(still);
  expect(await ringSize(page)).toBeLessThan(17.5);
  expect(await capsuleWidth(page)).toBe(34);

  await dispatch(page, "show-overlay", "transcribing");
  await expect(page.locator(".ov-capsule")).toHaveClass(/processing/);
  await expect(page.locator(".ov-orb-arc"), "no frozen arc").toHaveCount(0);
  expect(await page.evaluate(() => document.getAnimations().length)).toBe(0);
});

test("the meeting pill ticks without moving, turns its time red near the cap and gives way to dictation", async ({
  page,
}) => {
  const now = Date.parse("2026-10-17T10:00:00Z");
  const cap = 3 * 3600 * 1000;
  await page.clock.install({ time: now - 1_000 });
  await page.clock.pauseAt(now);
  await page.setViewportSize(MEETING_WINDOW);
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
  await expect(pill.locator(".ov-glyph.rec")).toHaveCount(1);
  const white = await pill.evaluate(
    (capsule) => getComputedStyle(capsule).color,
  );

  // A dictation takes the overlay over; the backend sends the pill back after.
  await dispatch(page, "show-overlay", "recording");
  await expect(pill).toHaveCount(0);
  await expect(page.locator(".ov-orb")).toBeVisible();

  const nearCap = now + 2_000 - (cap - 4 * 60 * 1000);
  await dispatch(page, "show-overlay-meeting", {
    started_at_ms: nearCap,
    stops_at_ms: nearCap + cap,
  });
  await expect(pill).toHaveText("2:56:00");
  await expect(pill).toHaveClass(/warning/);
  await expect(pill).toHaveCSS("color", "rgb(255, 69, 58)");
  expect(white).not.toBe("rgb(255, 69, 58)");
  // The orb keeps its white ring and red core.
  await expect(pill.locator(".ov-glyph")).toHaveCSS(
    "border-top-color",
    "rgba(255, 255, 255, 0.94)",
  );
});

test("after a sleep past the cap the pill reads the cap until the meeting stops", async ({
  page,
}) => {
  const now = Date.parse("2026-10-17T10:00:00Z");
  const cap = 3 * 3600 * 1000;
  await page.clock.install({ time: now - 1_000 });
  await page.clock.pauseAt(now);
  await page.setViewportSize(MEETING_WINDOW);
  await openOverlay(page);
  const startedAt = now - cap - 20 * 60 * 1000;

  await dispatch(page, "show-overlay-meeting", {
    started_at_ms: startedAt,
    stops_at_ms: startedAt + cap,
  });
  await expect(page.locator(".ov-meeting")).toHaveText("3:00:00");
});

const callLabels = (tray: Record<string, string>) => ({
  question: tray.callRecordQuestion,
  accept: tray.callRecord,
  decline: tray.callNotNow,
  position: "bottom",
});
const englishTray = trayStringsByLocale.find(
  ({ locale }) => locale === "en",
)!.tray;
const callAnswers = (page: Page) =>
  page.evaluate(() => window.overlayHarness.callAnswers());

test("a call prompt asks to record the meeting and sends back Record or Not now", async ({
  page,
}) => {
  await openCallPrompt(page);
  await dispatch(page, "show-overlay-call", callLabels(englishTray));

  const prompt = page.getByRole("alertdialog", {
    name: "Record this meeting?",
  });
  await expect(prompt).toBeVisible();
  await expect(prompt, "the question alone, without the app").toHaveText(
    "Record this meeting?Not nowRecord",
  );
  expect(await callAnswers(page), "nothing is answered on its own").toEqual([]);

  await prompt.getByRole("button", { name: "Record", exact: true }).click();
  await dispatch(page, "show-overlay-call", callLabels(englishTray));
  await prompt.getByRole("button", { name: "Not now", exact: true }).click();
  expect(await callAnswers(page)).toEqual([true, false]);
});

test("a dictation, its error or the meeting pill never hide the call prompt", async ({
  page,
}) => {
  await openCallPrompt(page);
  await dispatch(page, "show-overlay-call", callLabels(englishTray));
  const prompt = page.getByRole("alertdialog", {
    name: "Record this meeting?",
  });
  await expect(prompt).toBeVisible();

  const heard = [
    await dispatch(page, "show-overlay", "recording"),
    await dispatch(page, "recording-ready"),
    await dispatch(page, "show-overlay", "transcribing"),
    await dispatch(page, "show-overlay-error", "Transcription failed."),
    await dispatch(page, "show-overlay-copy", COPY_LABELS),
    await dispatch(page, "hide-overlay"),
    await dispatch(page, "show-overlay-meeting", {
      started_at_ms: 0,
      stops_at_ms: 10_800_000,
    }),
  ];
  expect(heard, "the prompt's window hears none of the overlay's").toEqual(
    Array(heard.length).fill(0),
  );
  await expect(prompt).toBeVisible();
  await prompt.getByRole("button", { name: "Record", exact: true }).click();
  expect(await callAnswers(page)).toEqual([true]);
});

test("a call prompt opens from the orb, and again when the call ends", async ({
  page,
}) => {
  await openCallPrompt(page);
  await page.evaluate(() => document.fonts.ready);
  for (const labels of [callLabels(englishTray), endedLabels(englishTray)]) {
    const widths = await widthsAfter(
      page,
      "show-overlay-call",
      labels,
      ".ov-call",
    );
    expect(
      widths[0],
      `${labels.kind}: width starts at the orb`,
    ).toBeLessThanOrEqual(40);
    expect(
      widths[widths.length - 1],
      `${labels.kind}: width ends fitted`,
    ).toBeGreaterThan(250);
  }
});

test("a call prompt shown before the page listened still appears", async ({
  page,
}) => {
  await openOverlay(page, {
    fixture: "call-prompt",
    readyOn: null,
    callPrompt: callLabels(englishTray),
  });

  await expect(
    page.getByRole("alertdialog", { name: "Record this meeting?" }),
  ).toBeVisible();
});

test("the call prompt sits on the overlay's side from its first frame", async ({
  page,
}) => {
  // Settings never answer: the side comes with the prompt.
  await openOverlay(page, {
    fixture: "call-prompt",
    readyOn: "hide-overlay-call",
    holdSettings: true,
  });
  const stage = page.locator(".ov-stage");

  await dispatch(page, "show-overlay-call", {
    ...callLabels(englishTray),
    position: "top",
  });
  await expect(stage).toHaveClass(/\btop\b/);

  // The overlay was moved while the prompt showed.
  await dispatch(page, "show-overlay-call", callLabels(englishTray));
  await expect(stage).toHaveClass(/\bbottom\b/);
});

test("the call prompt closes when the call ends", async ({ page }) => {
  await openCallPrompt(page);
  await dispatch(page, "show-overlay-call", callLabels(englishTray));
  await expect(page.getByRole("alertdialog")).toBeVisible();

  await dispatch(page, "hide-overlay-call");
  await expect(page.getByRole("alertdialog")).toHaveCount(0);
  expect(await callAnswers(page)).toEqual([]);
});

for (const { locale, tray } of trayStringsByLocale) {
  test(`the ${locale} call prompt fits its window, answers on one line`, async ({
    page,
  }) => {
    await openCallPrompt(page);
    await page.evaluate(() => document.fonts.ready);
    const labels = callLabels(tray);
    await dispatch(page, "show-overlay-call", labels);
    const prompt = page.getByRole("alertdialog", { name: labels.question });
    await expect(prompt).toBeVisible();
    await entranceFinished(page);
    const fitted = await fitCopyWindow(page);
    expect(fitted.width, "within the widest notice").toBeLessThanOrEqual(
      WIDEST_PILL + 32,
    );
    const layout = await prompt.evaluate((capsule) => {
      const box = capsule.getBoundingClientRect();
      const answers = [...capsule.querySelectorAll("button")].map((button) => {
        const rect = button.getBoundingClientRect();
        return {
          inside: rect.left >= box.left && rect.right <= box.right,
          oneLine: rect.height <= 28,
        };
      });
      const question = capsule.querySelector(".ov-call-question")!;
      const lines = document.createRange();
      lines.selectNodeContents(question);
      return {
        inWindow:
          box.left >= 0 &&
          box.top >= 0 &&
          box.right <= window.innerWidth &&
          box.bottom <= window.innerHeight,
        questionLines: new Set(
          [...lines.getClientRects()].map((line) => Math.round(line.top)),
        ).size,
        answers,
      };
    });
    expect(layout.inWindow, "capsule in its window").toBe(true);
    expect(layout.questionLines, "the question on one line").toBe(1);
    expect(layout.answers, "answers").toEqual([
      { inside: true, oneLine: true },
      { inside: true, oneLine: true },
    ]);
  });
}
