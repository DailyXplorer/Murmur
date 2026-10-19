import React from "react";
import ReactDOM from "react-dom/client";
import { emit } from "@tauri-apps/api/event";
import { mockIPC } from "@tauri-apps/api/mocks";
import type { AppSettings } from "../../src/bindings";
import "../../src/App.css";

type Deferred<T> = {
  promise: Promise<T>;
  reject: (reason?: unknown) => void;
  resolve: (value: T) => void;
};

const createDeferred = <T,>(): Deferred<T> => {
  let resolve!: (value: T) => void;
  let reject!: (reason?: unknown) => void;
  const promise = new Promise<T>((resolvePromise, rejectPromise) => {
    resolve = resolvePromise;
    reject = rejectPromise;
  });
  return { promise, reject, resolve };
};

const query = new URLSearchParams(window.location.search);
const appLanguage = query.get("lang") === "fr" ? "fr" : "en";
let codexConfigured = query.get("codex") === "configured";
/** A user who finished setup long ago and lost Accessibility since: the
 * first check finds it missing, the next one finds it granted again. */
const returning = query.get("returning") === "1";
let accessibilityRevoked = returning;
type CompletionFailureMode = "result" | "exception";

let nextCompletionFailure: CompletionFailureMode | null = null;
let nextSetupActionFailure: CompletionFailureMode | null = null;
let failNextInitialization = false;
const writes: string[] = [];
const calls: string[] = [];

type DeferredStatusRefresh = {
  codex: Deferred<{ signed_in: boolean; expired: boolean }>;
  codexConfigured: boolean;
  requested: boolean;
};

const deferredStatusRefreshes: DeferredStatusRefresh[] = [];
let activeDeferredStatusRefresh = 0;

const deferStatusRefresh = (): number => {
  deferredStatusRefreshes.push({
    codex: createDeferred(),
    codexConfigured,
    requested: false,
  });
  return deferredStatusRefreshes.length - 1;
};

const deferredStatusResponse = () => {
  const deferred = deferredStatusRefreshes[activeDeferredStatusRefresh];
  if (!deferred || deferred.requested) return null;

  deferred.requested = true;
  activeDeferredStatusRefresh += 1;
  return deferred.codex.promise;
};

const resolveStatusRefresh = (index: number) => {
  const deferred = deferredStatusRefreshes[index];
  if (!deferred) throw new Error(`Unknown deferred status refresh: ${index}`);
  deferred.codex.resolve({
    signed_in: deferred.codexConfigured,
    expired: false,
  });
};

const rejectStatusRefresh = (index: number) => {
  const deferred = deferredStatusRefreshes[index];
  if (!deferred) throw new Error(`Unknown deferred status refresh: ${index}`);
  deferred.codex.reject(new Error("Simulated provider status failure"));
};

let backendSettings: AppSettings = {
  app_language: appLanguage,
  onboarding_completed: returning,
  selected_language: "fr",
};

mockIPC(
  (command) => {
    calls.push(command);
    switch (command) {
      case "plugin:opener|open_url": {
        const failure = nextSetupActionFailure;
        nextSetupActionFailure = null;
        if (failure === "result") {
          return Promise.reject(
            "The operating system refused to open this resource",
          );
        }
        if (failure === "exception") {
          throw new Error("The operating system refused to open this resource");
        }
        return null;
      }
      case "get_app_settings":
      case "get_default_settings":
        return { ...backendSettings };
      case "check_custom_sounds":
        return { start: false, stop: false };
      case "get_codex_auth_status": {
        const response = deferredStatusResponse();
        if (response) return response;
        return { signed_in: codexConfigured, expired: false };
      }
      case "complete_onboarding": {
        const completionFailure = nextCompletionFailure;
        nextCompletionFailure = null;
        if (completionFailure === "result") {
          return Promise.reject(
            "The provider status changed while completing setup",
          );
        }
        if (completionFailure === "exception") {
          throw new Error("Configuration changed before setup completed");
        }
        if (!codexConfigured) {
          throw new Error("No ChatGPT/Codex session was found");
        }
        backendSettings = { ...backendSettings, onboarding_completed: true };
        writes.push("complete");
        return null;
      }
      case "get_available_microphones":
      case "get_available_output_devices":
        return [];
      case "plugin:macos-permissions|check_accessibility_permission":
        if (accessibilityRevoked) {
          accessibilityRevoked = false;
          return false;
        }
        return true;
      case "plugin:macos-permissions|check_microphone_permission":
        return true;
      case "get_microphone_authorization_status":
        return 3;
      case "initialize_enigo":
        if (failNextInitialization) {
          failNextInitialization = false;
          throw new Error("Keyboard automation is temporarily unavailable");
        }
        return null;
      case "initialize_shortcuts":
      case "show_main_window_command":
        return null;
      default:
        throw new Error(`Unexpected Tauri command: ${command}`);
    }
  },
  { shouldMockEvents: true },
);

declare global {
  interface Window {
    providerOnboardingFixture: {
      emitTranscriptionFailure: () => Promise<void>;
      deferStatusRefresh: () => number;
      failNextCompletion: (mode?: CompletionFailureMode) => void;
      failNextSetupAction: (mode: CompletionFailureMode) => void;
      failNextInitialization: () => void;
      calls: () => string[];
      initializationError: () => string | null;
      rejectStatusRefresh: (index: number) => void;
      resolveStatusRefresh: (index: number) => void;
      selectedLanguage: () => string | undefined;
      setCodexConfigured: (configured: boolean) => void;
      writes: () => string[];
    };
  }
}

let initializationError: string | null = null;

window.providerOnboardingFixture = {
  emitTranscriptionFailure: () =>
    emit("transcription-error", {
      kind: "rejected",
      detail: "The configured service rejected this audio.",
    }),
  deferStatusRefresh,
  failNextCompletion: (mode = "exception") => {
    nextCompletionFailure = mode;
  },
  failNextSetupAction: (mode) => {
    nextSetupActionFailure = mode;
  },
  failNextInitialization: () => {
    failNextInitialization = true;
  },
  calls: () => [...calls],
  initializationError: () => initializationError,
  rejectStatusRefresh,
  resolveStatusRefresh,
  selectedLanguage: () => backendSettings.selected_language,
  setCodexConfigured: (configured) => {
    codexConfigured = configured;
  },
  writes: () => [...writes],
};

void (async () => {
  try {
    const { default: App } = await import("../../src/App");
    ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
      <>
        <App />
        <span data-testid="provider-onboarding-mounted" />
      </>,
    );
  } catch (error) {
    initializationError =
      error instanceof Error ? error.message : String(error);
    console.error("Failed to render provider onboarding fixture:", error);
  }
})();
