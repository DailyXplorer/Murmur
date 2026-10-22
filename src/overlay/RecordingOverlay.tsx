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
  BAR_COUNT,
  BAR_MAX_PX,
  BAR_MIN_PX,
  barHeights,
  type Envelope,
  followPhrase,
  growStep,
  STATIC_HEIGHTS,
  voiceLevel,
} from "./waveform";

type View =
  | { kind: "hidden" }
  | { kind: "recording"; live: boolean }
  | { kind: "transcribing" }
  | { kind: "error"; message: string }
  | { kind: "copy"; labels: CopyLabels; copied: boolean }
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
      <span>{formatDuration(elapsed)}</span>
    </div>
  );
};

const CENTER = (BAR_COUNT - 1) / 2;

// The window renders no text until its first message, so without this the
// font would still be loading when the copy pill measures its fitted size.
// "aą" pulls in both the Latin and the Latin Extended subsets.
void document.fonts
  .load('1em "Instrument Sans Variable"', "aą")
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
  const capsuleRef = useRef<HTMLDivElement>(null);
  const waveRef = useRef<HTMLDivElement>(null);
  const copyRef = useRef<HTMLButtonElement>(null);
  const copyBodyRef = useRef<HTMLSpanElement>(null);
  const copyNoteRef = useRef<HTMLSpanElement>(null);
  const copyId = useId();

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

    // No text field had the focus, so the text is already on the clipboard.
    void listen<CopyLabels>("show-overlay-copy", (event) => {
      setView({ kind: "copy", labels: event.payload, copied: false });
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
  // keeps the bars smooth without re-rendering.
  useEffect(() => {
    const capsule = capsuleRef.current;
    const wave = waveRef.current;
    if (!animating || !capsule || !wave) return;
    const bars = Array.from(wave.children) as HTMLElement[];
    let heights = bars.map(() => BAR_MIN_PX);
    let envelope: Envelope = { level: 0, quietSeconds: 0 };
    let step = 0;
    let last = performance.now();
    let frame = requestAnimationFrame(function tick(now) {
      const dt = Math.min(0.1, (now - last) / 1000);
      last = now;
      const targets = barHeights(levelsRef.current);
      heights = heights.map((h, i) => approach(h, targets[i], dt));
      bars.forEach((bar, i) => {
        bar.style.setProperty("--s", (heights[i] / BAR_MAX_PX).toFixed(3));
      });
      envelope = followPhrase(envelope, voiceLevel(levelsRef.current), dt);
      const next = growStep(step, envelope.level);
      if (next !== step) {
        step = next;
        capsule.style.setProperty("--grow-step", String(step));
      }
      frame = requestAnimationFrame(tick);
    });
    return () => {
      cancelAnimationFrame(frame);
      bars.forEach((bar) => bar.style.removeProperty("--s"));
      capsule.style.removeProperty("--grow-step");
    };
  }, [animating]);

  // The note takes two lines, at the narrowest width that holds them, so the
  // pill widens with a longer translation or a wider font up to its widest
  // body. A note that needs more lines there keeps them, again as narrow as
  // they fit. A note only loses lines as it widens, so the narrowest width
  // is bisected. Layout sizes ignore the pop-in scale.
  const copyShowing = view.kind === "copy";
  const copyNote = view.kind === "copy" ? view.labels.note : null;
  useLayoutEffect(() => {
    const note = copyNoteRef.current;
    if (copyNote === null || !note) return;
    note.style.width = "";
    const style = getComputedStyle(note);
    const lines = () =>
      Math.round(
        (note.clientHeight -
          parseFloat(style.paddingTop) -
          parseFloat(style.paddingBottom)) /
          parseFloat(style.lineHeight),
      );
    const target = Math.max(2, lines());
    let [tooNarrow, wideEnough] = [0, Math.ceil(parseFloat(style.width))];
    while (wideEnough - tooNarrow > 1) {
      const width = Math.floor((tooNarrow + wideEnough) / 2);
      note.style.width = `${width}px`;
      if (lines() <= target) wideEnough = width;
      else tooNarrow = width;
    }
    note.style.width = `${wideEnough}px`;
  }, [copyNote]);

  // The copy pill's body sizes itself from its text, whatever font renders
  // it, and the window is fitted to it: only the webview can measure it.
  // The backend opens each notice at a default size, so every copy view,
  // even one replacing a showing pill, observes and reports again.
  const copyView = view.kind === "copy" ? view : null;
  useLayoutEffect(() => {
    const body = copyBodyRef.current;
    if (!copyView || !body) return;
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
  }, [copyView]);

  // The pill opens as the rest capsule and grows to fit its body. CSS cannot
  // transition to an auto size, so the fitted size is measured before the
  // first paint and the growth runs towards it. The body keeps its own size
  // and is clipped meanwhile, so the note never rewraps as the pill grows.
  useLayoutEffect(() => {
    const pill = copyRef.current;
    if (!copyShowing || !pill || reducedMotion) return;
    const style = getComputedStyle(pill);
    const growth = pill.animate(
      [
        {
          width: style.getPropertyValue("--ov-rest-w"),
          height: style.getPropertyValue("--ov-h"),
        },
        { width: `${pill.offsetWidth}px`, height: `${pill.offsetHeight}px` },
      ],
      { duration: 420, easing: "cubic-bezier(0.22, 1, 0.36, 1)" },
    );
    return () => growth.cancel();
  }, [copyShowing, reducedMotion]);

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
          ref={copyRef}
          type="button"
          dir="auto"
          className={`ov-capsule ov-copy${view.copied ? " copied" : ""}`}
          aria-labelledby={`${copyId}-action`}
          aria-describedby={`${copyId}-note`}
          onClick={copyAgain}
        >
          <span ref={copyBodyRef} className="ov-copy-body">
            <span
              ref={copyNoteRef}
              id={`${copyId}-note`}
              className="ov-copy-note"
            >
              {view.labels.note}
            </span>
            <span className="ov-copy-action">
              <Icon size={14} weight="bold" aria-hidden="true" />
              <span className="ov-copy-label">
                <span id={`${copyId}-action`} aria-live="polite">
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

  if (view.kind === "error") {
    return (
      <div className={`ov-stage ${position}`}>
        <div className="ov-capsule ov-error" role="alert">
          <span dir="auto">{view.message}</span>
        </div>
      </div>
    );
  }

  const phase =
    view.kind === "transcribing" ? "processing" : view.live ? "live" : "arming";

  return (
    <div className={`ov-stage ${position}`}>
      <div ref={capsuleRef} className={`ov-capsule ${phase}`}>
        <div ref={waveRef} className="ov-wave" aria-hidden="true">
          {STATIC_HEIGHTS.map((height, i) => (
            <i
              key={i}
              style={
                {
                  "--d": Math.abs(i - CENTER),
                  "--s": reducedMotion ? height / BAR_MAX_PX : undefined,
                } as React.CSSProperties
              }
            />
          ))}
        </div>
      </div>
    </div>
  );
};

export default RecordingOverlay;
