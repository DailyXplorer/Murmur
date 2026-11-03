import { expect, type Page } from "@playwright/test";
import type { MeetingNoticeLabels } from "../src/overlay/RecordingOverlay";

// CARD_MARGIN, ORB_OVERLAY_SIZE, OVERLAY_* and ERROR_OVERLAY_* in
// src-tauri/src/overlay.rs.
const CARD_MARGIN = 16;
export const ORB_WINDOW = { width: 66, height: 66 };
export const MEETING_WINDOW = { width: 140, height: 66 };
export const ERROR_WINDOW = { width: 336, height: 116 };

/** Opens the overlay page, or the call prompt's with `fixture`, with
 * Tauri's event bridge mocked; `readyOn` is an event the page listens to,
 * or `null` not to wait. `callPrompt` is what `get_call_prompt` answers. */
export async function openOverlay(
  page: Page,
  {
    holdSettings = false,
    copyError,
    fixture = "recording-overlay-listeners",
    readyOn = "mic-level",
    callPrompt = null,
    holdMeetingCopy = false,
    meetingNotice = null,
    holdPageEpoch = false,
  }: {
    holdSettings?: boolean;
    copyError?: string;
    fixture?: string;
    readyOn?: string | null;
    callPrompt?: unknown;
    holdMeetingCopy?: boolean;
    meetingNotice?: MeetingNoticeLabels | null;
    holdPageEpoch?: boolean;
  } = {},
) {
  const options = {
    hold: holdSettings,
    copyRejection: copyError,
    shownPrompt: callPrompt,
    holdMeetingCopy,
    meetingNotice,
    holdPageEpoch,
  };
  await page.addInitScript((configuration) => {
    const {
      hold,
      copyRejection,
      shownPrompt,
      holdMeetingCopy,
      meetingNotice,
      holdPageEpoch,
    } = configuration;
    const callbacks = new Map<number, (event: unknown) => unknown>();
    const listeners = new Map<number, string>();
    const pendingSettings: Array<(settings: unknown) => void> = [];
    let copyCalls = 0;
    const copyFits: Array<{ width: number; height: number }> = [];
    const callAnswers: string[] = [];
    let restoreCalls = 0;
    const meetingNoticeCalls: string[] = [];
    const pendingMeetingCopies: Array<() => void> = [];
    const pageRequests: string[] = [];
    const readyEpochs: unknown[] = [];
    const pageEpoch = "7";
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
      callAnswers: () => callAnswers,
      restoreCalls: () => restoreCalls,
      meetingNoticeCalls: () => meetingNoticeCalls,
      releaseMeetingCopy: () =>
        pendingMeetingCopies.splice(0).forEach((resolve) => resolve()),
      pageRequests: () => pageRequests,
      readyEpochs: () => readyEpochs,
    };

    window.__TAURI_INTERNALS__ = {
      invoke: (
        command: string,
        args?: {
          event?: string;
          handler?: number;
          width?: number;
          height?: number;
          answer?: string;
          payload?: unknown;
        },
      ) => {
        if (command === "plugin:event|listen") {
          listeners.set(args?.handler ?? 0, args?.event ?? "");
          return Promise.resolve(args?.handler);
        }
        if (command === "plugin:event|unlisten") return Promise.resolve();
        if (
          command === "plugin:event|emit_to" &&
          args?.event === "meeting-notice-page-request"
        ) {
          if (typeof args.payload !== "string") {
            throw new Error("The overlay page nonce is missing");
          }
          pageRequests.push(args.payload);
          if (!holdPageEpoch) {
            window.overlayHarness.dispatch("meeting-notice-page-epoch", {
              nonce: args.payload,
              epoch: pageEpoch,
            });
          }
          return Promise.resolve();
        }
        if (
          command === "plugin:event|emit_to" &&
          args?.event === "meeting-notice-listener-ready"
        ) {
          readyEpochs.push(args.payload);
          if (meetingNotice && args.payload === pageEpoch) {
            window.overlayHarness.dispatch(
              "show-overlay-meeting-notice",
              meetingNotice,
            );
          }
          return Promise.resolve();
        }
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
        if (
          command === "open_meeting_notice" ||
          command === "copy_meeting_notice"
        ) {
          meetingNoticeCalls.push(command);
          if (command === "copy_meeting_notice") {
            if (copyRejection) return Promise.reject(copyRejection);
            if (holdMeetingCopy) {
              return new Promise<void>((resolve) =>
                pendingMeetingCopies.push(resolve),
              );
            }
          }
          return Promise.resolve(null);
        }
        if (command === "fit_copy_notice" || command === "fit_call_prompt") {
          copyFits.push({ width: args?.width ?? 0, height: args?.height ?? 0 });
          return Promise.resolve(null);
        }
        if (command === "answer_call_prompt") {
          callAnswers.push(args?.answer ?? "");
          return Promise.resolve(null);
        }
        if (command === "restore_cancelled_dictation") {
          restoreCalls += 1;
          return Promise.resolve(null);
        }
        if (command === "get_call_prompt") {
          return Promise.resolve(shownPrompt);
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

  await page.goto(`/tests/fixtures/${fixture}.html`);
  if (readyOn === null) return;
  await expect
    .poll(() =>
      page.evaluate(
        (event) => window.overlayHarness.dispatch(event, [0]),
        readyOn,
      ),
    )
    .toBe(1);
}

/** Opens the call prompt's own window. */
export const openCallPrompt = (page: Page) =>
  openOverlay(page, { fixture: "call-prompt", readyOn: "hide-overlay-call" });

export const dispatch = (page: Page, event: string, payload?: unknown) =>
  page.evaluate(
    ([name, value]) => window.overlayHarness.dispatch(name as string, value),
    [event, payload] as const,
  );

/** Resizes the page the way `fit_copy_notice` and `fit_call_prompt` resize
 * their windows: the last capsule size the copy pill or call prompt
 * measured plus CARD_MARGIN around it. */
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

/** Sends `event`, then reads the laid-out width of the capsule `selector`
 * finds on every animation frame for 0.9 s, from the first frame it shows
 * `shows` (a pill replacing another of the same capsule): a pill that opens
 * from the orb starts at its 34 points. */
export const widthsAfter = (
  page: Page,
  event: string,
  payload: unknown,
  selector: string,
  shows = "",
) =>
  page.evaluate(
    ([name, value, capsule, text]) =>
      new Promise<number[]>((resolve) => {
        window.overlayHarness.dispatch(name as string, value);
        const seen: number[] = [];
        const start = performance.now();
        requestAnimationFrame(function sample(now) {
          const pill = document.querySelector<HTMLElement>(capsule as string);
          if (pill?.textContent?.includes(text as string)) {
            seen.push(pill.offsetWidth);
          }
          if (now - start < 900) requestAnimationFrame(sample);
          else resolve(seen);
        });
      }),
    [event, payload, selector, shows] as const,
  );

// Drawn sizes include the capsule's pop-in scale until it ends.
export const entranceFinished = (page: Page) =>
  page
    .locator(".ov-capsule")
    .evaluate((capsule) =>
      Promise.all(capsule.getAnimations().map((entry) => entry.finished)),
    );
