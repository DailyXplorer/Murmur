import { ArrowCounterClockwiseIcon } from "@phosphor-icons/react/dist/csr/ArrowCounterClockwise";
import { CheckIcon } from "@phosphor-icons/react/dist/csr/Check";
import { CopyIcon } from "@phosphor-icons/react/dist/csr/Copy";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import React, {
  useEffect,
  useId,
  useLayoutEffect,
  useRef,
  useState,
  useSyncExternalStore,
} from "react";
import "./RecordingOverlay.css";
import { commands } from "@/bindings";
import { CAP_WARNING_MS, formatDuration } from "@/lib/meetings/format";
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
  | { kind: "meeting"; startedAtMs: number; stopsAtMs: number };

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

const reducedMotionQuery = window.matchMedia(
  "(prefers-reduced-motion: reduce)",
);
const subscribeReducedMotion = (onChange: () => void) => {
  reducedMotionQuery.addEventListener("change", onChange);
  return () => reducedMotionQuery.removeEventListener("change", onChange);
};
const getReducedMotion = () => reducedMotionQuery.matches;

const RecordingOverlay: React.FC = () => {
  const [view, setView] = useState<View>({ kind: "hidden" });
  const [position, setPosition] = useState<"top" | "bottom">("bottom");
  const reducedMotion = useSyncExternalStore(
    subscribeReducedMotion,
    getReducedMotion,
  );
  const levelsRef = useRef<number[]>([]);
  const ringRef = useRef<SVGPathElement>(null);
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

  // The copy pill's body, and the warning pill's, sizes itself from its
  // text, whatever font renders it, and the window is fitted to it: only the
  // webview can measure it. The backend opens each notice at a default size,
  // so every such view, even one replacing a showing one, observes and
  // reports again.
  const fittedView =
    view.kind === "copy" || view.kind === "warning" || view.kind === "cancelled"
      ? view
      : null;
  const pillShowing = fittedView !== null;
  useLayoutEffect(() => {
    const body = noticeBodyRef.current;
    if (!fittedView || !body) return;
    if (fittedView.kind === "warning") hugWrappedLines(body);
    const observer = new ResizeObserver(([entry]) => {
      const [box] = entry.borderBoxSize;
      commands
        .fitCopyNotice(box.inlineSize, box.blockSize)
        .catch((error: unknown) => {
          console.warn("Fitting the copy pill's window failed:", error);
        });
    });
    observer.observe(body);
    return () => observer.disconnect();
  }, [fittedView]);

  // The pill opens as the orb's circle and grows to fit its body. CSS cannot
  // transition to an auto size, so the fitted size is measured before the
  // first paint and the growth runs towards it. The body keeps its own size
  // and is clipped meanwhile, so the note never rewraps as the pill grows.
  useLayoutEffect(() => {
    const pill = noticeBodyRef.current?.parentElement;
    if (!pillShowing || !pill || reducedMotion) return;
    const style = getComputedStyle(pill);
    const growth = pill.animate(
      [
        {
          width: style.getPropertyValue("--ov-orb"),
          height: style.getPropertyValue("--ov-orb"),
        },
        { width: `${pill.offsetWidth}px`, height: `${pill.offsetHeight}px` },
      ],
      { duration: 420, easing: "cubic-bezier(0.22, 1, 0.36, 1)" },
    );
    return () => growth.cancel();
  }, [pillShowing, reducedMotion]);

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
        <div className="ov-capsule ov-warning" role="status" dir="auto">
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
        <div className="ov-capsule ov-error" role="alert">
          <Glyph kind="alert" />
          <span className="ov-error-text" dir="auto">
            {view.message}
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
          {phase === "processing" && (
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
