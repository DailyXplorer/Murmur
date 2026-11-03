import { ArrowCounterClockwiseIcon } from "@phosphor-icons/react/dist/csr/ArrowCounterClockwise";
import { CheckIcon } from "@phosphor-icons/react/dist/csr/Check";
import { CopyIcon } from "@phosphor-icons/react/dist/csr/Copy";
import { XIcon } from "@phosphor-icons/react/dist/csr/X";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import React, {
  useEffect,
  useId,
  useLayoutEffect,
  useRef,
  useState,
} from "react";
import "./RecordingOverlay.css";
import { commands } from "@/bindings";
import {
  CAP_WARNING_MS,
  formatDuration,
  formatMeetingDate,
} from "@/lib/meetings/format";
import { openFromOrb, useReducedMotion } from "./openFromOrb";
import {
  approach,
  ORB_RADIUS,
  ORB_SIZE,
  orbPath,
  RESTING_PATH,
  voiceLevel,
} from "./waveform";

type View =
  | { kind: "hidden" }
  | { kind: "recording"; live: boolean }
  | { kind: "transcribing" }
  | { kind: "error"; message: string }
  | { kind: "copy"; labels: CopyLabels; copied: boolean }
  | { kind: "warning"; message: string }
  | { kind: "cancelled"; labels: CancelledLabels }
  | {
      kind: "meetingNotice";
      labels: MeetingNoticeLabels;
      copiedUntil: number | null;
    }
  | { kind: "meeting"; startedAtMs: number; stopsAtMs: number };

export interface MeetingNoticeLabels {
  tone: "ready" | "failed";
  headline: string;
  detail: string | null;
  open: string;
  copy: string | null;
  copied: string;
  close: string;
  default_name: { started_at_ms: number; locale: string } | null;
  meeting_id: number | null;
}

const sameMeetingNotice = (
  current: MeetingNoticeLabels,
  shown: MeetingNoticeLabels,
) => current.meeting_id !== null && current.meeting_id === shown.meeting_id;

/** Payload of `show-overlay-meeting`, from `overlay::MeetingPill`. */
interface MeetingPill {
  started_at_ms: number;
  stops_at_ms: number;
}

/** Payload of `show-overlay-copy`, from `overlay::CopyNoticeLabels`. */
interface CopyLabels {
  copy: string;
  copied: string;
  note: string;
}

/** Payload of `show-overlay-cancelled`, from `overlay::CancelledNoticeLabels`. */
interface CancelledLabels {
  note: string;
  restore: string;
}

/** Ticks once a second while a meeting records. */
const MeetingPillView: React.FC<{ startedAtMs: number; stopsAtMs: number }> = ({
  startedAtMs,
  stopsAtMs,
}) => {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    const timer = window.setInterval(() => setNow(Date.now()), 1000);
    return () => window.clearInterval(timer);
  }, []);
  const warning = now >= stopsAtMs - CAP_WARNING_MS;
  // Woken past the cap, the meeting stops within a tick: never read past it.
  const elapsed = Math.min(now, stopsAtMs) - startedAtMs;
  return (
    <div
      className={`ov-capsule ov-meeting${warning ? " warning" : ""}`}
      role="timer"
    >
      <Glyph kind="rec" />
      <span>{formatDuration(elapsed)}</span>
    </div>
  );
};

/** The orb, small, at the start of a pill: what the pill is about, in the
 * dictation's own shape. */
const Glyph: React.FC<{
  kind: "ring" | "done" | "off" | "dim" | "alert" | "rec";
}> = ({ kind }) => <span className={`ov-glyph ${kind}`} aria-hidden="true" />;

/** A balanced paragraph that wraps keeps the width it wrapped at, wider than
 * its lines; narrowed to its longest line, the pill hugs its text and its
 * padding stays even on both sides. */
const hugWrappedLines = (body: HTMLElement) => {
  for (const text of body.querySelectorAll<HTMLElement>(".ov-copy-note")) {
    text.style.width = "";
    const range = document.createRange();
    range.selectNodeContents(text);
    const lines = [...range.getClientRects()];
    if (new Set(lines.map((line) => Math.round(line.top))).size < 2) continue;
    const left = Math.min(...lines.map((line) => line.left));
    const right = Math.max(...lines.map((line) => line.right));
    // Line boxes are drawn sizes, scaled by the pill's pop-in; offsetWidth
    // is the laid-out one.
    const scale = text.getBoundingClientRect().width / text.offsetWidth;
    text.style.width = `${Math.ceil((right - left) / scale)}px`;
  }
};

// The window renders no text until its first message, so without this the
// font would still be loading when the copy pill measures its fitted size.
void document.fonts
  .load('1em "Instrument Sans Variable"')
  .catch(() => undefined);

const RecordingOverlay: React.FC = () => {
  const [view, setView] = useState<View>({ kind: "hidden" });
  const [position, setPosition] = useState<"top" | "bottom">("bottom");
  const reducedMotion = useReducedMotion();
  const levelsRef = useRef<number[]>([]);
  const ringRef = useRef<SVGPathElement>(null);
  const pillRef = useRef<HTMLElement | null>(null);
  const setPill = (pill: HTMLElement | null) => {
    pillRef.current = pill;
  };
  const noticeBodyRef = useRef<HTMLSpanElement>(null);
  const noticeId = useId();

  useEffect(() => {
    let isUnmounted = false;
    const unlisteners: UnlistenFn[] = [];
    const storeUnlistener = (unlisten: UnlistenFn) => {
      if (isUnmounted) {
        unlisten();
      } else {
        unlisteners.push(unlisten);
      }
    };

    const refreshPosition = () => {
      void commands
        .getAppSettings()
        .then((settings) => {
          if (settings.status === "ok") {
            setPosition(
              settings.data.overlay_position === "top" ? "top" : "bottom",
            );
          }
        })
        .catch(() => undefined);
    };

    void listen<string>("show-overlay", (event) => {
      levelsRef.current = [];
      // Applied before any await: an error or hide that arrives while the
      // settings load must not be overwritten by this older show.
      setView(
        event.payload === "transcribing"
          ? { kind: "transcribing" }
          : { kind: "recording", live: false },
      );
      refreshPosition();
    }).then(storeUnlistener, () => undefined);

    void listen<MeetingPill>("show-overlay-meeting", (event) => {
      setView({
        kind: "meeting",
        startedAtMs: event.payload.started_at_ms,
        stopsAtMs: event.payload.stops_at_ms,
      });
      refreshPosition();
    }).then(storeUnlistener, () => undefined);

    void listen<MeetingNoticeLabels>("show-overlay-meeting-notice", (event) => {
      let labels = event.payload;
      if (labels.default_name) {
        const date = formatMeetingDate(
          labels.default_name.started_at_ms,
          labels.default_name.locale,
        );
        labels = {
          ...labels,
          headline: labels.headline.replace("{{date}}", date),
          detail: labels.detail?.replace("{{date}}", date) ?? null,
        };
      }
      setView((current) => ({
        kind: "meetingNotice",
        labels,
        copiedUntil:
          current.kind === "meetingNotice" &&
          labels.copy !== null &&
          sameMeetingNotice(current.labels, labels)
            ? current.copiedUntil
            : null,
      }));
      refreshPosition();
    }).then(storeUnlistener, () => undefined);

    // The backend localizes the message, so it matches the notification
    // shown when the overlay is off.
    void listen<string>("show-overlay-error", (event) => {
      setView({ kind: "error", message: event.payload });
    }).then(storeUnlistener, () => undefined);

    // No text field had the focus; the text goes on the clipboard only when
    // the pill is clicked.
    void listen<CopyLabels>("show-overlay-copy", (event) => {
      setView({ kind: "copy", labels: event.payload, copied: false });
      refreshPosition();
    }).then(storeUnlistener, () => undefined);

    // The microphone went away during a dictation that pasted its text, or
    // had none: the warning shows alone, the way the copy pill does.
    void listen<string>("show-overlay-warning", (event) => {
      setView({ kind: "warning", message: event.payload });
      refreshPosition();
    }).then(storeUnlistener, () => undefined);

    // A dictation was cancelled: Restore runs it through the pipeline.
    void listen<CancelledLabels>("show-overlay-cancelled", (event) => {
      setView({ kind: "cancelled", labels: event.payload });
      refreshPosition();
    }).then(storeUnlistener, () => undefined);

    void listen("hide-overlay", () => {
      setView({ kind: "hidden" });
    }).then(storeUnlistener, () => undefined);

    void listen("recording-ready", () => {
      setView((current) =>
        current.kind === "recording" ? { ...current, live: true } : current,
      );
    }).then(storeUnlistener, () => undefined);

    void listen<number[]>("mic-level", (event) => {
      levelsRef.current = event.payload;
    }).then(storeUnlistener, () => undefined);

    return () => {
      isUnmounted = true;
      unlisteners.forEach((unlisten) => unlisten());
    };
  }, []);

  const animating = view.kind === "recording" && view.live && !reducedMotion;

  // Levels arrive at ~30 Hz; easing them every animation frame outside React
  // keeps the outline smooth without re-rendering.
  useEffect(() => {
    const ring = ringRef.current;
    if (!animating || !ring) return;
    let level = 0;
    let last = performance.now();
    let frame = requestAnimationFrame(function tick(now) {
      const dt = Math.min(0.1, (now - last) / 1000);
      last = now;
      level = approach(level, voiceLevel(levelsRef.current), dt);
      ring.setAttribute("d", orbPath(level, now / 1000));
      ring.style.fillOpacity = (0.1 + 0.25 * level).toFixed(3);
      frame = requestAnimationFrame(tick);
    });
    return () => {
      cancelAnimationFrame(frame);
      ring.setAttribute("d", RESTING_PATH);
      ring.style.removeProperty("fill-opacity");
    };
  }, [animating]);

  // Only the webview can measure the body. Native notices start at a default
  // size, so even a notice replacing a showing one must report its fit again.
  const fittedView =
    view.kind === "copy" ||
    view.kind === "warning" ||
    view.kind === "cancelled" ||
    view.kind === "meetingNotice"
      ? view
      : null;
  useLayoutEffect(() => {
    const body = noticeBodyRef.current;
    if (!fittedView || !body) return;
    if (fittedView.kind === "warning") hugWrappedLines(body);
    const observer = new ResizeObserver(([entry]) => {
      const [box] = entry.borderBoxSize;
      commands
        .fitCopyNotice(box.inlineSize, box.blockSize)
        .catch((error: unknown) => {
          console.warn("Fitting the notice pill's window failed:", error);
        });
    });
    observer.observe(body);
    return () => observer.disconnect();
  }, [fittedView]);

  // Each pill opens from the orb, again when another kind replaces it (the
  // microphone warning after the copy pill), not when Copy turns to Copied.
  const pillKind =
    fittedView?.kind ?? (view.kind === "error" ? view.kind : null);
  useLayoutEffect(() => {
    const pill = pillRef.current;
    if (!pillKind || !pill || reducedMotion) return;
    const growth = openFromOrb(pill);
    return () => growth.cancel();
  }, [pillKind, reducedMotion]);

  const copiedUntil = view.kind === "meetingNotice" ? view.copiedUntil : null;
  useEffect(() => {
    if (copiedUntil === null) return;
    const timer = window.setTimeout(
      () => {
        setView((current) =>
          current.kind === "meetingNotice" &&
          current.copiedUntil === copiedUntil
            ? { ...current, copiedUntil: null }
            : current,
        );
      },
      Math.max(0, copiedUntil - Date.now()),
    );
    return () => window.clearTimeout(timer);
  }, [copiedUntil]);

  if (view.kind === "hidden") return null;

  if (view.kind === "meeting") {
    return (
      <div className={`ov-stage ${position}`}>
        <MeetingPillView
          startedAtMs={view.startedAtMs}
          stopsAtMs={view.stopsAtMs}
        />
      </div>
    );
  }

  if (view.kind === "meetingNotice") {
    const copyMeeting = () => {
      void commands
        .copyMeetingNotice()
        .then((result) => {
          if (result.status === "error") throw new Error(result.error);
          setView((current) =>
            current.kind === "meetingNotice" &&
            current.labels.copy !== null &&
            sameMeetingNotice(current.labels, view.labels)
              ? { ...current, copiedUntil: Date.now() + 1500 }
              : current,
          );
        })
        .catch((error: unknown) => {
          console.warn("Copying the meeting notice failed:", error);
        });
    };
    const openMeeting = () => {
      void commands.openMeetingNotice().catch((error: unknown) => {
        console.warn("Opening the meeting notice failed:", error);
      });
    };
    const dismissMeeting = () => {
      void commands.dismissMeetingNotice().catch((error: unknown) => {
        console.warn("Dismissing the meeting notice failed:", error);
      });
    };
    const copied = view.copiedUntil !== null;
    const Icon = copied ? CheckIcon : CopyIcon;
    return (
      <div className={`ov-stage ${position}`}>
        <div
          ref={setPill}
          className={`ov-capsule ov-meeting-notice${view.labels.tone === "failed" ? " ov-warning" : ""}`}
          role="status"
          dir="auto"
        >
          <span ref={noticeBodyRef} className="ov-meeting-notice-body">
            <Glyph kind={view.labels.tone === "ready" ? "done" : "off"} />
            <span className="ov-meeting-notice-text">
              <span className="ov-meeting-notice-headline">
                {view.labels.headline}
              </span>
              {view.labels.detail !== null && (
                <span className="ov-meeting-notice-detail">
                  {view.labels.detail}
                </span>
              )}
            </span>
            <span className="ov-meeting-notice-actions">
              <button
                type="button"
                className="ov-call-button"
                onClick={openMeeting}
              >
                {view.labels.open}
              </button>
              {view.labels.copy !== null && (
                <button
                  type="button"
                  className="ov-call-button ov-copy-action"
                  aria-labelledby={`${noticeId}-action`}
                  onClick={copyMeeting}
                >
                  <Icon
                    size={14}
                    weight="bold"
                    aria-hidden="true"
                    data-icon={copied ? "check" : "copy"}
                  />
                  <span className="ov-copy-label">
                    <span id={`${noticeId}-action`} aria-live="polite">
                      {copied ? view.labels.copied : view.labels.copy}
                    </span>
                    <span className="ov-copy-label-spare" aria-hidden="true">
                      {copied ? view.labels.copy : view.labels.copied}
                    </span>
                  </span>
                </button>
              )}
            </span>
            <button
              type="button"
              className="ov-call-button ov-meeting-notice-close"
              aria-label={view.labels.close}
              onClick={dismissMeeting}
            >
              <XIcon size={12} weight="bold" aria-hidden="true" />
            </button>
          </span>
        </div>
      </div>
    );
  }

  if (view.kind === "copy") {
    const copyAgain = () => {
      void commands
        .copyNoticeAgain()
        .then((result) => {
          if (result.status !== "ok") throw result.error;
          setView((current) =>
            current.kind === "copy" ? { ...current, copied: true } : current,
          );
        })
        .catch((error: unknown) => {
          console.warn("Copying the dictation again failed:", error);
        });
    };
    const Icon = view.copied ? CheckIcon : CopyIcon;
    return (
      <div className={`ov-stage ${position}`}>
        <button
          ref={setPill}
          type="button"
          dir="auto"
          className={`ov-capsule ov-copy${view.copied ? " copied" : ""}`}
          aria-labelledby={`${noticeId}-action`}
          aria-describedby={`${noticeId}-note`}
          onClick={copyAgain}
        >
          <span ref={noticeBodyRef} className="ov-copy-body">
            <Glyph kind={view.copied ? "done" : "ring"} />
            <span id={`${noticeId}-note`} className="ov-copy-note">
              {view.labels.note}
            </span>
            <span className="ov-copy-action">
              <Icon size={14} weight="bold" aria-hidden="true" />
              <span className="ov-copy-label">
                <span id={`${noticeId}-action`} aria-live="polite">
                  {view.copied ? view.labels.copied : view.labels.copy}
                </span>
                <span className="ov-copy-label-spare" aria-hidden="true">
                  {view.copied ? view.labels.copy : view.labels.copied}
                </span>
              </span>
            </span>
          </span>
        </button>
      </div>
    );
  }

  if (view.kind === "cancelled") {
    const restore = () => {
      commands.restoreCancelledDictation().catch((error: unknown) => {
        console.warn("Restoring the cancelled dictation failed:", error);
      });
    };
    return (
      <div className={`ov-stage ${position}`}>
        <button
          ref={setPill}
          type="button"
          dir="auto"
          className="ov-capsule ov-copy"
          aria-labelledby={`${noticeId}-action`}
          aria-describedby={`${noticeId}-note`}
          onClick={restore}
        >
          <span ref={noticeBodyRef} className="ov-copy-body">
            <Glyph kind="dim" />
            <span id={`${noticeId}-note`} className="ov-copy-note">
              {view.labels.note}
            </span>
            <span className="ov-copy-action">
              <ArrowCounterClockwiseIcon
                size={14}
                weight="bold"
                aria-hidden="true"
              />
              <span id={`${noticeId}-action`} className="ov-copy-label">
                {view.labels.restore}
              </span>
            </span>
          </span>
        </button>
      </div>
    );
  }

  if (view.kind === "warning") {
    return (
      <div className={`ov-stage ${position}`}>
        <div
          ref={setPill}
          className="ov-capsule ov-warning"
          role="status"
          dir="auto"
        >
          <span ref={noticeBodyRef} className="ov-copy-body">
            <Glyph kind="off" />
            <span className="ov-copy-note ov-copy-warning">{view.message}</span>
          </span>
        </div>
      </div>
    );
  }

  if (view.kind === "error") {
    return (
      <div className={`ov-stage ${position}`}>
        <div ref={setPill} className="ov-capsule ov-error" role="alert">
          <span className="ov-error-body">
            <Glyph kind="alert" />
            <span className="ov-error-text" dir="auto">
              {view.message}
            </span>
          </span>
        </div>
      </div>
    );
  }

  const phase =
    view.kind === "transcribing" ? "processing" : view.live ? "live" : "arming";

  return (
    <div className={`ov-stage ${position}`}>
      <div className={`ov-capsule ov-dictation ${phase}`}>
        <svg
          className="ov-orb"
          viewBox={`0 0 ${ORB_SIZE} ${ORB_SIZE}`}
          aria-hidden="true"
        >
          <path ref={ringRef} className="ov-orb-ring" d={RESTING_PATH} />
          {phase === "processing" && !reducedMotion && (
            <circle
              className="ov-orb-arc"
              cx={ORB_SIZE / 2}
              cy={ORB_SIZE / 2}
              r={ORB_RADIUS}
            />
          )}
        </svg>
      </div>
    </div>
  );
};

export default RecordingOverlay;
