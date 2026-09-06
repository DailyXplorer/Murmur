import React from "react";
import ReactDOM from "react-dom/client";
import { emit } from "@tauri-apps/api/event";
import { mockIPC } from "@tauri-apps/api/mocks";
import type { AppSettings, TranscriptionProvider } from "../../src/bindings";
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
const configuredProvider: TranscriptionProvider =
  query.get("provider") === "gemini" ? "gemini" : "codex";
let codexConfigured = query.get("codex") === "configured";
let geminiInstalled = query.get("gemini") === "configured";
let geminiConfigured = query.get("gemini") === "configured";
type CompletionFailureMode = "result" | "exception";

let nextCompletionFailure: CompletionFailureMode | null = null;
let failNextInitialization = false;
let failNextProviderChange = false;
const writes: string[] = [];
const calls: string[] = [];

type DeferredStatusRefresh = {
  codex: Deferred<{ signed_in: boolean }>;
  codexConfigured: boolean;
  gemini: Deferred<{ installed: boolean; signed_in: boolean }>;
  geminiConfigured: boolean;
  geminiInstalled: boolean;
  requested: Set<"codex" | "gemini">;
};

const deferredStatusRefreshes: DeferredStatusRefresh[] = [];
let activeDeferredStatusRefresh = 0;

const deferStatusRefresh = (): number => {
  const deferred: DeferredStatusRefresh = {
    codex: createDeferred(),
    codexConfigured,
    gemini: createDeferred(),
    geminiConfigured,
    geminiInstalled,
    requested: new Set(),
  };
  deferredStatusRefreshes.push(deferred);
  return deferredStatusRefreshes.length - 1;
};

const deferredStatusResponse = (
  provider: "codex" | "gemini",
): Promise<
  { signed_in: boolean } | { installed: boolean; signed_in: boolean }
> | null => {
  const deferred = deferredStatusRefreshes[activeDeferredStatusRefresh];
  if (!deferred || deferred.requested.has(provider)) return null;

  deferred.requested.add(provider);
  if (deferred.requested.size === 2) activeDeferredStatusRefresh += 1;
  return provider === "codex"
    ? deferred.codex.promise
    : deferred.gemini.promise;
};

const resolveStatusRefresh = (index: number) => {
  const deferred = deferredStatusRefreshes[index];
  if (!deferred) throw new Error(`Unknown deferred status refresh: ${index}`);
  deferred.codex.resolve({ signed_in: deferred.codexConfigured });
  deferred.gemini.resolve({
    installed: deferred.geminiInstalled,
    signed_in: deferred.geminiConfigured,
  });
};

const rejectStatusRefresh = (index: number) => {
  const deferred = deferredStatusRefreshes[index];
  if (!deferred) throw new Error(`Unknown deferred status refresh: ${index}`);
  const error = new Error("Simulated provider status failure");
  deferred.codex.reject(error);
  deferred.gemini.reject(error);
};

let backendSettings: AppSettings = {
  app_language: appLanguage,
  onboarding_completed: false,
  selected_language: "fr",
  transcription_provider: configuredProvider,
};

const updateProvider = (provider: unknown) => {
  if (failNextProviderChange) {
    failNextProviderChange = false;
    throw new Error("The provider setting could not be saved");
  }
  if (provider !== "codex" && provider !== "gemini") {
    throw new Error("Unexpected transcription provider");
  }
  backendSettings = { ...backendSettings, transcription_provider: provider };
  writes.push(`provider:${provider}`);
};

mockIPC(
  (command, args) => {
    calls.push(command);
    switch (command) {
      case "get_app_settings":
      case "get_default_settings":
        return { ...backendSettings };
      case "check_custom_sounds":
        return { start: false, stop: false };
      case "get_codex_auth_status": {
        const response = deferredStatusResponse("codex");
        if (response) return response;
        return { signed_in: codexConfigured };
      }
      case "get_gemini_status": {
        const response = deferredStatusResponse("gemini");
        if (response) return response;
        return { installed: geminiInstalled, signed_in: geminiConfigured };
      }
      case "change_transcription_provider_setting":
        updateProvider(args?.provider);
        return null;
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
        const provider = backendSettings.transcription_provider;
        const providerIsConfigured =
          provider === "codex" ? codexConfigured : geminiConfigured;
        if (!providerIsConfigured) {
          throw new Error("The selected provider is not configured");
        }
        backendSettings = { ...backendSettings, onboarding_completed: true };
        writes.push("complete");
        return null;
      }
      case "get_available_microphones":
      case "get_available_output_devices":
        return [];
      case "plugin:macos-permissions|check_accessibility_permission":
      case "plugin:macos-permissions|check_microphone_permission":
        return true;
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
      failNextInitialization: () => void;
      failNextProviderChange: () => void;
      calls: () => string[];
      initializationError: () => string | null;
      provider: () => TranscriptionProvider | undefined;
      rejectStatusRefresh: (index: number) => void;
      resolveStatusRefresh: (index: number) => void;
      selectedLanguage: () => string | undefined;
      setCodexConfigured: (configured: boolean) => void;
      setGeminiConfigured: (configured: boolean) => void;
      writes: () => string[];
    };
  }
}

let initializationError: string | null = null;

window.providerOnboardingFixture = {
  emitTranscriptionFailure: () =>
    emit("transcription-error", "The configured service rejected this audio."),
  deferStatusRefresh,
  failNextCompletion: (mode = "exception") => {
    nextCompletionFailure = mode;
  },
  failNextInitialization: () => {
    failNextInitialization = true;
  },
  failNextProviderChange: () => {
    failNextProviderChange = true;
  },
  calls: () => [...calls],
  initializationError: () => initializationError,
  provider: () => backendSettings.transcription_provider,
  rejectStatusRefresh,
  resolveStatusRefresh,
  selectedLanguage: () => backendSettings.selected_language,
  setCodexConfigured: (configured) => {
    codexConfigured = configured;
  },
  setGeminiConfigured: (configured) => {
    geminiInstalled = configured;
    geminiConfigured = configured;
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
