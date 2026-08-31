import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import React, { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import "./RecordingOverlay.css";
import { commands } from "@/bindings";
import i18n, { syncLanguageFromSettings } from "@/i18n";
import { getLanguageDirection } from "@/lib/utils/rtl";
import { XIcon } from "@phosphor-icons/react/dist/csr/X";

type OverlayState = "recording" | "transcribing";

const WAVE_BARS = 9;

const RecordingOverlay: React.FC = () => {
  const { t } = useTranslation();
  const [isVisible, setIsVisible] = useState(false);
  const [state, setState] = useState<OverlayState>("recording");
  const [captureReady, setCaptureReady] = useState(false);
  const [levels, setLevels] = useState<number[]>(Array(WAVE_BARS).fill(0));
  const [position, setPosition] = useState<"top" | "bottom">("bottom");
  const smoothedLevelsRef = useRef<number[]>(Array(16).fill(0));
  const direction = getLanguageDirection(i18n.language);

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

    void listen("show-overlay", async (event) => {
      const overlayState = event.payload as OverlayState;
      if (overlayState === "recording") {
        setCaptureReady(false);
        smoothedLevelsRef.current = Array(16).fill(0);
        setLevels(Array(WAVE_BARS).fill(0));
      }

      await syncLanguageFromSettings();
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
      setState(overlayState);
      setIsVisible(true);
    }).then(storeUnlistener, () => undefined);

    void listen("hide-overlay", () => {
      setIsVisible(false);
      setCaptureReady(false);
    }).then(storeUnlistener, () => undefined);

    void listen("recording-ready", () => {
      setCaptureReady(true);
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

  const listeningRow = (
    <div className="sbase">
      <div className="sbase-l">
        <span className={`sdot ${captureReady ? "ready" : "arming"}`} />
      </div>
      {waveform}
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

  return (
    <div
      dir={direction}
      className={`ov-stage ${position} ov-fade ${isVisible ? "show" : ""}`}
    >
      <div
        className={`scard compact ${working && isVisible ? "cworking" : ""}`}
      >
        {working ? workingRow(workLabel) : listeningRow}
      </div>
    </div>
  );
};

export default RecordingOverlay;
