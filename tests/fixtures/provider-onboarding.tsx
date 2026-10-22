import React from "react";
import ReactDOM from "react-dom/client";
import { emit } from "@tauri-apps/api/event";
import { mockIPC } from "@tauri-apps/api/mocks";
import type {
  AppSettings,
  DictationStyle,
  Engine,
  LocalModelStatus,
} from "../../src/bindings";
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
const appLanguage = query.get("lang") ?? "en";
let codexConfigured = query.get("codex") === "configured";
/** A user who finished setup long ago and lost Accessibility since: the
 * first check finds it missing, the next one finds it granted again. */
const returning = query.get("returning") === "1";
let accessibilityRevoked = returning;
/** `permissions=missing`: Accessibility is never granted, so the
 * permissions step stays on screen. */
const permissionsMissing = query.get("permissions") === "missing";

/** The manifest's download: Apple silicon, or `mac=intel`. */
const MODEL_BYTES = query.get("mac") === "intel" ? 739_508_576 : 505_044_208;
const MODELS = {
  missing: { state: "missing" },
  downloading: {
    state: "downloading",
    done_bytes: 191_916_799,
    total_bytes: MODEL_BYTES,
  },
  preparing: { state: "preparing" },
  ready: { state: "ready" },
  failed: { state: "failed", reason: { kind: "no_internet" } },
  // The default: a Mac without the local model gets today's Codex setup.
  unsupported: { state: "unsupported", support: "os_too_old" },
} satisfies Record<string, LocalModelStatus["state"]>;
type ModelName = keyof typeof MODELS;
const modelStatus = (name: ModelName): LocalModelStatus => ({
  state: MODELS[name],
  size_bytes: MODEL_BYTES,
  kept_bytes: 0,
});
let model = modelStatus((query.get("model") as ModelName) ?? "unsupported");

const setModel = (name: ModelName) => {
  model = modelStatus(name);
  return emit("local-model-status", model);
};

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

/** `slowStatus=1`: the first status check waits for `resolveStatusRefresh(0)`. */
if (query.get("slowStatus") === "1") deferStatusRefresh();

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
  // `cleanup=on`: a connected account leads to the style step.
  ai_cleanup_enabled: query.get("cleanup") === "on",
  app_language: appLanguage,
  dictation_style: (query.get("style") ?? "normal") as DictationStyle,
  onboarding_completed: returning,
  selected_language: "fr",
  transcription_engine: returning ? "cloud" : "local",
};

const engines: Engine[] = [];

mockIPC(
  (command, args) => {
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
        // As the backend: a Mac that cannot run the local model finishes in
        // the cloud, and the cloud needs a session.
        const engine: Engine =
          model.state.state === "unsupported"
            ? "cloud"
            : (backendSettings.transcription_engine ?? "local");
        if (engine === "cloud" && !codexConfigured) {
          throw new Error("No ChatGPT/Codex session was found");
        }
        backendSettings = {
          ...backendSettings,
          onboarding_completed: true,
          transcription_engine: engine,
          local_stt_card_dismissed: true,
        };
        writes.push("complete");
        return null;
      }
      case "get_local_model_status":
        return model;
      case "change_transcription_engine_setting": {
        const engine = (args as { engine: Engine }).engine;
        backendSettings = { ...backendSettings, transcription_engine: engine };
        engines.push(engine);
        if (engine === "local" && model.state.state !== "ready") {
          void setModel("downloading");
        }
        return null;
      }
      case "download_local_model":
        void setModel("downloading");
        return null;
      case "change_dictation_style_setting":
        backendSettings = {
          ...backendSettings,
          dictation_style: (args as { style: DictationStyle }).style,
        };
        return null;
      case "get_available_microphones":
      case "get_available_output_devices":
        return [];
      case "plugin:macos-permissions|check_accessibility_permission":
        if (permissionsMissing) return false;
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
      engine: () => Engine | undefined;
      engines: () => Engine[];
      initializationError: () => string | null;
      setModel: (name: ModelName) => Promise<void>;
      rejectStatusRefresh: (index: number) => void;
      resolveStatusRefresh: (index: number) => void;
      selectedLanguage: () => string | undefined;
      setCodexConfigured: (configured: boolean) => void;
      style: () => DictationStyle | undefined;
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
  engine: () => backendSettings.transcription_engine,
  engines: () => [...engines],
  initializationError: () => initializationError,
  setModel,
  rejectStatusRefresh,
  resolveStatusRefresh,
  selectedLanguage: () => backendSettings.selected_language,
  setCodexConfigured: (configured) => {
    codexConfigured = configured;
  },
  style: () => backendSettings.dictation_style,
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
