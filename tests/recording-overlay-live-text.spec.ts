import { expect, test, type Locator, type Page } from "@playwright/test";
import {
  dispatch,
  entranceFinished,
  LIVE_TEXT_WINDOW,
  openOverlay,
  ORB_WINDOW,
  widthsAfter,
} from "./overlayHarness";

const TEXT = "dictation-live-text";
const LONG =
  "First of all thank you for coming, we will go through the plan for the next quarter and then take questions at the very end";
const ARABIC =
  "مرحبا بكم جميعا في هذا الاجتماع وسوف نتحدث اليوم عن خطة العمل للربع القادم ثم نجيب عن الأسئلة في النهاية";

const capsule = (page: Page) => page.locator(".ov-capsule");
const words = (page: Page) => page.locator(".ov-live-words");
const clip = (page: Page) => page.locator(".ov-live-text");

const box = (locator: Locator) =>
  locator.evaluate((element) => {
    const { left, right, width, height } = element.getBoundingClientRect();
    return { left, right, width, height };
  });

/** Where `word` is drawn (its last occurrence), and the box that clips the
 * line. */
const wordBox = (page: Page, word: string) =>
  page.evaluate((searched) => {
    const line = document.querySelector(".ov-live-words")!;
    const text = line.firstChild as Text;
    const at = text.data.lastIndexOf(searched);
    const range = document.createRange();
    range.setStart(text, at);
    range.setEnd(text, at + searched.length);
    const drawn = range.getBoundingClientRect();
    const clipped = document
      .querySelector(".ov-live-text")!
      .getBoundingClientRect();
    return {
      left: drawn.left,
      right: drawn.right,
      clipLeft: clipped.left,
      clipRight: clipped.right,
    };
  }, word);

const fade = (page: Page) =>
  clip(page).evaluate((element) => getComputedStyle(element).maskImage);

const record = async (page: Page, window = LIVE_TEXT_WINDOW) => {
  await page.setViewportSize(window);
  await openOverlay(page);
  await page.evaluate(() => document.fonts.ready);
  await dispatch(page, "show-overlay", "recording");
  await dispatch(page, "recording-ready");
  await expect(capsule(page)).toHaveClass(/live/);
  await entranceFinished(page);
};

test("with no words the dictation is the 34-point orb, in the wide window too", async ({
  page,
}) => {
  await record(page);

  expect(await box(capsule(page))).toEqual({
    left: 219,
    right: 253,
    width: 34,
    height: 34,
  });
  await expect(capsule(page)).toHaveClass("ov-capsule ov-dictation live");
  await expect(capsule(page)).not.toHaveAttribute("dir");
  await expect(clip(page)).toHaveCount(0);
  await expect(page.locator(".ov-stage")).toHaveText("");

  await dispatch(page, "show-overlay", "transcribing");
  await expect(capsule(page)).toHaveClass("ov-capsule ov-dictation processing");
  expect((await box(capsule(page))).width).toBe(34);
});

test("the words show beside the orb and the pill grows with them, centred", async ({
  page,
}) => {
  await record(page);

  await dispatch(page, TEXT, "Bonjour");
  await expect(words(page)).toHaveText("Bonjour");
  const first = await box(capsule(page));
  expect(first.width).toBeGreaterThan(34 + 40);
  expect(first.height).toBe(34);

  // No growth to wait for: the width is final on the first frame.
  const widths = await widthsAfter(
    page,
    TEXT,
    "Bonjour à tous",
    ".ov-capsule",
    "Bonjour à tous",
  );
  expect(new Set(widths).size).toBe(1);
  const second = await box(capsule(page));
  expect(second.width).toBeGreaterThan(first.width + 20);
  expect(second.width).toBeLessThan(440);
  expect(second.left + second.right).toBeCloseTo(LIVE_TEXT_WINDOW.width, 1);

  // The orb first, unchanged, then the line, with the notices' end margin.
  const orb = await box(page.locator(".ov-orb"));
  expect(orb.left).toBeCloseTo(second.left, 1);
  expect(orb.width).toBe(34);
  const last = await wordBox(page, "tous");
  expect(last.clipLeft).toBeCloseTo(orb.right, 1);
  expect(second.right - last.right).toBeCloseTo(14, 0);

  await expect(clip(page)).toHaveAttribute("aria-hidden", "true");
  await expect(clip(page)).not.toHaveAttribute("data-clipped");
  expect(await fade(page)).toBe("none");
  await expect(page.locator(".ov-orb-ring")).toHaveCount(1);
});

test("a long dictation keeps its newest words in the pill and clips the oldest", async ({
  page,
}) => {
  await record(page);

  await dispatch(page, TEXT, LONG);
  await expect(words(page)).toHaveText(LONG);

  expect(await box(capsule(page))).toEqual({
    left: 16,
    right: 456,
    width: 440,
    height: 34,
  });
  const newest = await wordBox(page, "very end");
  expect(newest.left).toBeGreaterThan(newest.clipLeft);
  expect(newest.right).toBeLessThanOrEqual(newest.clipRight);
  expect(456 - newest.right).toBeCloseTo(14, 0);
  const oldest = await wordBox(page, "First of all");
  expect(oldest.right, "out of the box, behind the orb").toBeLessThan(
    oldest.clipLeft,
  );

  await expect(clip(page)).toHaveAttribute("data-clipped", "ltr");
  expect(await fade(page)).toContain("linear-gradient(to right");

  // The fade goes with the clipping.
  await dispatch(page, TEXT, "First of all");
  await expect(words(page)).toHaveText("First of all");
  await expect(clip(page)).not.toHaveAttribute("data-clipped");
  expect(await fade(page)).toBe("none");
});

test("the pill is never wider than 440 points, whatever the window", async ({
  page,
}) => {
  await record(page, { width: 800, height: 66 });

  await dispatch(page, TEXT, LONG);
  await expect(words(page)).toHaveText(LONG);

  expect(await box(capsule(page))).toEqual({
    left: 180,
    right: 620,
    width: 440,
    height: 34,
  });
});

test("the words stay, dimmed, while the dictation transcribes", async ({
  page,
}) => {
  await record(page);
  await dispatch(page, TEXT, "Bonjour à tous");
  await expect(words(page)).toHaveText("Bonjour à tous");
  const opacity = () =>
    clip(page).evaluate((element) => getComputedStyle(element).opacity);
  expect(await opacity()).toBe("1");

  await dispatch(page, "show-overlay", "transcribing");

  await expect(capsule(page)).toHaveClass(/processing/);
  await expect(words(page)).toHaveText("Bonjour à tous");
  await expect.poll(opacity).toBe("0.55");
  await expect(page.locator(".ov-orb-arc")).toHaveCount(1);

  // The final text comes once the recording has stopped.
  await dispatch(page, TEXT, "Bonjour à tous.");
  await expect(words(page)).toHaveText("Bonjour à tous.");
});

test("a new recording and a hide clear the words", async ({ page }) => {
  await record(page);
  await dispatch(page, TEXT, "Bonjour à tous");
  await expect(words(page)).toHaveText("Bonjour à tous");

  await dispatch(page, "show-overlay", "recording");
  await expect(clip(page)).toHaveCount(0);
  expect((await box(capsule(page))).width).toBe(34);

  await dispatch(page, TEXT, "Encore");
  await expect(words(page)).toHaveText("Encore");
  await dispatch(page, "hide-overlay");
  await expect(page.locator(".ov-stage")).toHaveCount(0);

  // Restore transcribes with no recording before it.
  await dispatch(page, "show-overlay", "transcribing");
  await expect(capsule(page)).toHaveClass(/processing/);
  await entranceFinished(page);
  await expect(clip(page)).toHaveCount(0);
  expect((await box(capsule(page))).width).toBe(34);
});

test("words that come when no dictation shows are dropped", async ({
  page,
}) => {
  await page.setViewportSize(LIVE_TEXT_WINDOW);
  await openOverlay(page);

  expect(await dispatch(page, TEXT, "Trop tôt")).toBe(1);
  await dispatch(page, "show-overlay", "recording");
  await expect(capsule(page)).toHaveClass(/arming/);
  await expect(clip(page)).toHaveCount(0);

  await dispatch(page, TEXT, "Bonjour");
  await expect(words(page)).toHaveText("Bonjour");
  await dispatch(page, "show-overlay-cancelled", {
    note: "Dictation cancelled",
    restore: "Restore",
  });
  await expect(page.getByRole("button", { name: "Restore" })).toBeVisible();
  await dispatch(page, TEXT, "Trop tard");
  await expect(page.locator(".ov-stage")).not.toContainText("Trop tard");

  await dispatch(page, "show-overlay", "transcribing");
  await expect(capsule(page)).toHaveClass(/processing/);
  await expect(clip(page)).toHaveCount(0);
});

test("Arabic words run from the right and keep their newest in the pill", async ({
  page,
}) => {
  await record(page);

  await dispatch(page, TEXT, "مرحبا بكم");
  await expect(words(page)).toHaveText("مرحبا بكم");
  await expect(capsule(page)).toHaveAttribute("dir", "auto");
  const short = await box(capsule(page));
  expect(short.left + short.right).toBeCloseTo(LIVE_TEXT_WINDOW.width, 1);
  // The orb is first on the right, where the line starts.
  expect((await box(page.locator(".ov-orb"))).right).toBeCloseTo(
    short.right,
    1,
  );
  await expect(clip(page)).not.toHaveAttribute("data-clipped");

  await dispatch(page, TEXT, ARABIC);
  await expect(words(page)).toHaveText(ARABIC);
  expect((await box(capsule(page))).width).toBe(440);
  const newest = await wordBox(page, "في النهاية");
  expect(newest.left).toBeGreaterThanOrEqual(newest.clipLeft);
  expect(newest.right).toBeLessThan(newest.clipRight);
  expect(newest.left - 16).toBeCloseTo(14, 0);
  const oldest = await wordBox(page, "مرحبا بكم");
  expect(oldest.left, "out of the box, behind the orb").toBeGreaterThan(
    oldest.clipRight,
  );

  await expect(clip(page)).toHaveAttribute("data-clipped", "rtl");
  expect(await fade(page)).toContain("linear-gradient(to left");
});

test("reduced motion still shows the words", async ({ page }) => {
  await page.emulateMedia({ reducedMotion: "reduce" });
  await record(page);

  await dispatch(page, TEXT, LONG);
  await expect(words(page)).toHaveText(LONG);
  expect((await box(capsule(page))).width).toBe(440);
  const newest = await wordBox(page, "very end");
  expect(newest.right).toBeLessThanOrEqual(newest.clipRight);

  await dispatch(page, "show-overlay", "transcribing");
  await expect(words(page)).toHaveText(LONG);
  expect(
    await clip(page).evaluate((element) => {
      const style = getComputedStyle(element);
      return [style.opacity, style.transitionDuration];
    }),
  ).toEqual(["0.55", "0s"]);
  await expect(page.locator(".ov-orb-arc")).toHaveCount(0);
});

test("at most the last 400 characters are rendered", async ({ page }) => {
  await record(page);
  const numbered = Array.from({ length: 300 }, (_, index) => index).join(" ");
  expect(numbered.length).toBeGreaterThan(1000);

  await dispatch(page, TEXT, numbered);
  await expect(words(page)).toHaveText(numbered.slice(-400), {
    useInnerText: false,
  });
  expect(await words(page).evaluate((line) => line.textContent)).toBe(
    numbered.slice(-400),
  );

  // Cut between code points: no half of a pair is left at the start.
  await dispatch(page, TEXT, "😀".repeat(450));
  await expect
    .poll(() => words(page).evaluate((line) => line.textContent))
    .toBe("😀".repeat(400));
});

test("in a window with no room for words the pill stays the orb", async ({
  page,
}) => {
  await record(page, ORB_WINDOW);

  await dispatch(page, TEXT, LONG);
  await expect(words(page)).toHaveText(LONG);

  expect(await box(capsule(page))).toEqual({
    left: 16,
    right: 50,
    width: 34,
    height: 34,
  });
});
