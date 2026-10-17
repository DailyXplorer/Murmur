import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import React, {
  useEffect,
  useRef,
  useState,
  useSyncExternalStore,
} from "react";
import "./RecordingOverlay.css";
import { commands } from "@/bindings";
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
  | { kind: "error"; message: string };

const CENTER = (BAR_COUNT - 1) / 2;

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

    void listen<string>("show-overlay", (event) => {
      levelsRef.current = [];
      // Applied before any await: an error or hide that arrives while the
      // settings load must not be overwritten by this older show.
      setView(
        event.payload === "transcribing"
          ? { kind: "transcribing" }
          : { kind: "recording", live: false },
      );

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
    }).then(storeUnlistener, () => undefined);

    // The backend localizes the message, so it matches the notification
    // shown when the overlay is off.
    void listen<string>("show-overlay-error", (event) => {
      setView({ kind: "error", message: event.payload });
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

  if (view.kind === "hidden") return null;

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
