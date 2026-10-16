import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import React, { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import "./RecordingOverlay.css";
import { commands } from "@/bindings";
import i18n, { syncLanguageFromSettings } from "@/i18n";
import { formatKeyCombination } from "@/lib/utils/keyboard";
import { getLanguageDirection } from "@/lib/utils/rtl";
import { LockSimpleIcon } from "@phosphor-icons/react/dist/csr/LockSimple";
import { XIcon } from "@phosphor-icons/react/dist/csr/X";

type OverlayState = "recording" | "transcribing" | "error";

const WAVE_BARS = 9;
const HANDS_FREE_HINT_MS = 3000;

const RecordingOverlay: React.FC = () => {
  const { t } = useTranslation();
  const [isVisible, setIsVisible] = useState(false);
  const [state, setState] = useState<OverlayState>("recording");
  const [captureReady, setCaptureReady] = useState(false);
  const [errorMessage, setErrorMessage] = useState("");
  const [levels, setLevels] = useState<number[]>(Array(WAVE_BARS).fill(0));
  const [position, setPosition] = useState<"top" | "bottom">("bottom");
  const [stopShortcut, setStopShortcut] = useState("");
  const [handsFree, setHandsFree] = useState(false);
  const [showHandsFreeHint, setShowHandsFreeHint] = useState(false);
  const [limitEndsAt, setLimitEndsAt] = useState<number | null>(null);
  const [now, setNow] = useState(() => Date.now());
  const smoothedLevelsRef = useRef<number[]>(Array(16).fill(0));
  const direction = getLanguageDirection(i18n.language);

  useEffect(() => {
    if (!showHandsFreeHint) return;
    const timer = window.setTimeout(
      () => setShowHandsFreeHint(false),
      HANDS_FREE_HINT_MS,
    );
    return () => window.clearTimeout(timer);
  }, [showHandsFreeHint]);

  useEffect(() => {
    if (limitEndsAt === null) return;
    const timer = window.setInterval(() => setNow(Date.now()), 1000);
    return () => window.clearInterval(timer);
  }, [limitEndsAt]);

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

    void listen("show-overlay", (event) => {
      const overlayState = event.payload as OverlayState;
      if (overlayState === "recording") {
        setCaptureReady(false);
        setHandsFree(false);
        setShowHandsFreeHint(false);
        setLimitEndsAt(null);
        smoothedLevelsRef.current = Array(16).fill(0);
        setLevels(Array(WAVE_BARS).fill(0));
      }
      // Applied before any await: an error or hide that arrives while the
      // settings load must not be overwritten by this older show.
      setState(overlayState);
      setIsVisible(true);

      void syncLanguageFromSettings();
      void commands
        .getAppSettings()
        .then((settings) => {
          if (settings.status === "ok") {
            setPosition(
              settings.data.overlay_position === "top" ? "top" : "bottom",
            );
            const binding = settings.data.bindings?.transcribe?.current_binding;
            setStopShortcut(binding ? formatKeyCombination(binding) : "");
          }
        })
        .catch(() => undefined);
    }).then(storeUnlistener, () => undefined);

    // The backend localizes the message, so it matches the notification
    // shown when the overlay is off.
    void listen<string>("show-overlay-error", (event) => {
      setErrorMessage(event.payload);
      setState("error");
      setIsVisible(true);
    }).then(storeUnlistener, () => undefined);

    void listen("hide-overlay", () => {
      setIsVisible(false);
      setCaptureReady(false);
    }).then(storeUnlistener, () => undefined);

    void listen("recording-ready", () => {
      setCaptureReady(true);
    }).then(storeUnlistener, () => undefined);

    void listen("overlay-hands-free", () => {
      setHandsFree(true);
      setShowHandsFreeHint(true);
    }).then(storeUnlistener, () => undefined);

    void listen<number>("recording-limit-warning", (event) => {
      setNow(Date.now());
      setLimitEndsAt(Date.now() + event.payload * 1000);
    }).then(storeUnlistener, () => undefined);

    void listen<number[]>("mic-level", (event) => {
      const newLevels = event.payload as number[];
      const smoothed = smoothedLevelsRef.current.map((prev, i) => {
        const target = newLevels[i] || 0;
        return prev * 0.7 + target * 0.3;
      });
      smoothedLevelsRef.current = smoothed;
      setLevels(smoothed.slice(0, WAVE_BARS));
    }).then(storeUnlistener, () => undefined);

    return () => {
      isUnmounted = true;
      unlisteners.forEach((unlisten) => unlisten());
    };
  }, []);

  if (!isVisible) return null;

  const waveform = (
    <div className={`swave ${captureReady ? "ready" : "arming"}`}>
      {levels.map((v, i) => (
        <i
          key={i}
          style={{
            height: `${Math.max(3, Math.min(18, 3 + Math.pow(v, 0.7) * 15))}px`,
          }}
        />
      ))}
    </div>
  );

  const cancelBtn = (
    <button
      className="sx"
      aria-label={t("tray.cancel")}
      onClick={() => commands.cancelOperation()}
    >
      <XIcon size={9} aria-hidden="true" />
    </button>
  );

  const stopHint = t("overlay.handsFreeHint", { shortcut: stopShortcut });
  const secondsLeft =
    limitEndsAt === null
      ? null
      : Math.max(0, Math.ceil((limitEndsAt - now) / 1000));
  let note: string | null = null;
  if (secondsLeft !== null) {
    note = t("overlay.limitWarning", { seconds: secondsLeft });
  } else if (handsFree && showHandsFreeHint && stopShortcut) {
    note = stopHint;
  }

  const listeningRow = (
    <div className="sbase">
      <div className="sbase-l">
        {handsFree ? (
          <span
            className={`slock ${captureReady ? "ready" : "arming"}`}
            role="img"
            aria-label={t("overlay.handsFree")}
            title={stopShortcut ? stopHint : t("overlay.handsFree")}
          >
            <LockSimpleIcon size={11} weight="fill" aria-hidden="true" />
          </span>
        ) : (
          <span className={`sdot ${captureReady ? "ready" : "arming"}`} />
        )}
      </div>
      {note === null ? (
        waveform
      ) : (
        <span className="swork-label" role="status">
          {note}
        </span>
      )}
      <div className="sbase-r">{cancelBtn}</div>
    </div>
  );

  const workingRow = (label: string) => (
    <div className="sbase">
      <div className="sbase-l">
        <span className="sspinner" />
      </div>
      <span className="swork-label">{label}</span>
      <div className="sbase-r">{cancelBtn}</div>
    </div>
  );

  const working = state === "transcribing";
  const workLabel = t("overlay.transcribing");

  if (state === "error") {
    return (
      <div
        dir={direction}
        className={`ov-stage ${position} ov-fade ${isVisible ? "show" : ""}`}
      >
        <div className="scard compact cerror" role="alert">
          <div className="serror">
            <span className="serror-dot" aria-hidden="true" />
            <span className="serror-text">{errorMessage}</span>
          </div>
        </div>
      </div>
    );
  }

  return (
    <div
      dir={direction}
      className={`ov-stage ${position} ov-fade ${isVisible ? "show" : ""}`}
    >
      <div
        className={`scard compact ${(working || note !== null) && isVisible ? "cworking" : ""}`}
      >
        {working ? workingRow(workLabel) : listeningRow}
      </div>
    </div>
  );
};

export default RecordingOverlay;
