import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import React, {
  useEffect,
  useId,
  useLayoutEffect,
  useRef,
  useState,
} from "react";
import "./RecordingOverlay.css";
import { commands, type CallPromptLabels } from "@/bindings";

// The window renders no text until its first message, so without this the
// font would still be loading when the prompt measures its fitted size.
void document.fonts
  .load('1em "Instrument Sans Variable"')
  .catch(() => undefined);

/** "Record this meeting?", in a window of its own: only the call watcher
 * shows or hides it, so nothing the overlay shows takes its place. */
const CallPromptOverlay: React.FC = () => {
  const [labels, setLabels] = useState<CallPromptLabels | null>(null);
  const bodyRef = useRef<HTMLSpanElement>(null);
  const id = useId();

  useEffect(() => {
    let isUnmounted = false;
    // An event heard since the page asked is newer than the answer.
    let heard = false;
    const unlisteners: UnlistenFn[] = [];
    const storeUnlistener = (unlisten: UnlistenFn) => {
      if (isUnmounted) {
        unlisten();
      } else {
        unlisteners.push(unlisten);
      }
    };

    const listening = [
      listen<CallPromptLabels>("show-overlay-call", (event) => {
        heard = true;
        setLabels(event.payload);
      }).then(storeUnlistener, () => undefined),
      listen("hide-overlay-call", () => {
        heard = true;
        setLabels(null);
      }).then(storeUnlistener, () => undefined),
    ];

    // A prompt shown while the page loaded was sent before it listened.
    void Promise.all(listening)
      .then(() => commands.getCallPrompt())
      .then((shown) => {
        if (shown && !heard && !isUnmounted) setLabels(shown);
      })
      .catch(() => undefined);

    return () => {
      isUnmounted = true;
      unlisteners.forEach((unlisten) => unlisten());
    };
  }, []);

  // The body sizes itself from its text, whatever font renders it, and the
  // window is fitted to it: only the webview can measure it.
  useLayoutEffect(() => {
    const body = bodyRef.current;
    if (!labels || !body) return;
    const observer = new ResizeObserver(([entry]) => {
      const [box] = entry.borderBoxSize;
      commands
        .fitCallPrompt(box.inlineSize, box.blockSize)
        .catch((error: unknown) => {
          console.warn("Fitting the call prompt's window failed:", error);
        });
    });
    observer.observe(body);
    return () => observer.disconnect();
  }, [labels]);

  if (!labels) return null;

  const answer = (accept: boolean) => {
    void commands.answerCallPrompt(accept).catch((error: unknown) => {
      console.warn("Answering the call prompt failed:", error);
    });
  };

  return (
    <div className={`ov-stage ${labels.position}`}>
      <div
        role="alertdialog"
        dir="auto"
        className="ov-capsule ov-call"
        aria-labelledby={`${id}-question`}
        aria-describedby={`${id}-app`}
      >
        <span ref={bodyRef} className="ov-call-body">
          <span className="ov-call-text">
            <span id={`${id}-question`} className="ov-call-question">
              {labels.question}
            </span>
            <span id={`${id}-app`} className="ov-call-app">
              {labels.app}
            </span>
          </span>
          <button
            type="button"
            className="ov-call-button"
            onClick={() => answer(false)}
          >
            {labels.decline}
          </button>
          <button
            type="button"
            className="ov-call-button accept"
            onClick={() => answer(true)}
          >
            {labels.accept}
          </button>
        </span>
      </div>
    </div>
  );
};

export default CallPromptOverlay;
