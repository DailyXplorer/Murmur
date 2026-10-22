import {
  useCallback,
  useEffect,
  useRef,
  useState,
  type ReactNode,
} from "react";
import { toast, Toaster } from "sonner";
import { useTranslation } from "react-i18next";
import { listen } from "@tauri-apps/api/event";
import {
  checkAccessibilityPermission,
  checkMicrophonePermission,
} from "tauri-plugin-macos-permissions-api";
import { RecordingErrorEvent } from "./lib/types/events";
import "./App.css";
import AccessibilityPermissions from "./components/AccessibilityPermissions";
import Footer from "./components/footer";
import {
  AccessibilityOnboarding,
  DoneOnboarding,
  LocalModelOnboarding,
  ProviderOnboarding,
  StyleOnboarding,
} from "./components/onboarding";
import { ErrorBoundary } from "./components/ErrorBoundary";
import { Sidebar, SidebarSection, SECTIONS_CONFIG } from "./components/Sidebar";
import { WhatsNewGate } from "./components/whats-new";
import { useSettings } from "./hooks/useSettings";
import { useLocalModel } from "./hooks/useLocalModel";
import { useSettingsStore } from "./stores/settingsStore";
import {
  commands,
  events,
  type DictationStyle,
  type Engine,
  type LocalModelStatus,
} from "@/bindings";
import { LocalSttCard } from "./components/LocalSttCard";
import { LocalModelPill } from "./components/LocalModelPill";

/** A new install goes model → permissions → account → style (when AI
 * cleanup can run) → finish (on this Mac only) → done. `returning`: setup
 * was finished before, and a permission went missing since; granting it
 * again leads back to the app. `engine` is the one chosen in the model
 * step; the cloud makes the account required. */
type OnboardingState =
  | { kind: "model"; status: LocalModelStatus }
  | { kind: "permissions"; returning: true }
  | { kind: "permissions"; returning: false; engine: Engine }
  | { kind: "account"; engine: Engine }
  | { kind: "style"; next: "finish" | "done" }
  | { kind: "finish" }
  | { kind: "done" };

/** A style only changes what AI cleanup writes, so the step shows only when
 * cleanup is on and has an account to run with. */
const stepAfterAccount = (
  engine: Engine | undefined,
  cleanupCanRun: boolean,
): OnboardingState => {
  const next = engine === "local" ? "finish" : "done";
  return cleanupCanRun ? { kind: "style", next } : { kind: next };
};

/** Mirrors `FailureKind` in src-tauri/src/transcription_error.rs. */
type FailureKind =
  | "network"
  | "timeout"
  | "rate_limited"
  | "server_error"
  | "session_expired"
  | "signed_out"
  | "too_large"
  | "rejected"
  | "local_not_ready"
  | "local_failed"
  | "language_needs_cloud"
  | "other";

interface TranscriptionErrorEvent {
  kind: FailureKind;
  detail: string;
}

/** The same messages the overlay and notifications show. */
const FAILURE_MESSAGE_KEYS: Record<FailureKind, string> = {
  network: "tray.errorNetwork",
  timeout: "tray.errorTimeout",
  rate_limited: "tray.errorRateLimited",
  server_error: "tray.errorServer",
  session_expired: "tray.errorSessionExpired",
  signed_out: "tray.errorSignedOut",
  too_large: "tray.errorTooLarge",
  rejected: "tray.errorTranscription",
  local_not_ready: "tray.errorLocalNotReady",
  local_failed: "tray.errorLocalFailed",
  language_needs_cloud: "tray.errorLanguageNeedsCloud",
  other: "tray.errorTranscription",
};

const initializeKeyboardAutomation = async () => {
  const [enigoResult, shortcutsResult] = await Promise.all([
    commands.initializeEnigo(),
    commands.initializeShortcuts(),
  ]);

  if (enigoResult.status === "error") {
    throw new Error(enigoResult.error);
  }
  if (shortcutsResult.status === "error") {
    throw new Error(shortcutsResult.error);
  }
};

const revealMainWindowForPermissions = async () => {
  try {
    await commands.showMainWindowCommand();
  } catch (e) {
    console.warn("Failed to show main window for permission onboarding:", e);
  }
};

const renderSettingsContent = (section: SidebarSection) => {
  const ActiveComponent = SECTIONS_CONFIG[section].component;
  return <ActiveComponent />;
};

/** Settings window shell, including first-run onboarding. */
function App() {
  const { t } = useTranslation();
  const [onboardingState, setOnboardingState] =
    useState<OnboardingState | null>(null);
  const [currentSection, setCurrentSection] =
    useState<SidebarSection>("general");
  const { settings, updateSetting, refreshSettings } = useSettings();
  const refreshAudioDevices = useSettingsStore(
    (state) => state.refreshAudioDevices,
  );
  const refreshOutputDevices = useSettingsStore(
    (state) => state.refreshOutputDevices,
  );
  const hasCompletedPostOnboardingInit = useRef(false);
  const localModel = useLocalModel();

  useEffect(() => {
    checkOnboardingStatus();
  }, []);

  // The finish step tells the user to hold the shortcut, so it must work.
  useEffect(() => {
    if (
      (onboardingState?.kind !== "done" &&
        onboardingState?.kind !== "finish") ||
      hasCompletedPostOnboardingInit.current
    ) {
      return;
    }

    let cancelled = false;

    const initializeAfterOnboarding = async () => {
      try {
        await initializeKeyboardAutomation();
        if (cancelled) {
          return;
        }
        hasCompletedPostOnboardingInit.current = true;
        refreshAudioDevices();
        refreshOutputDevices();
      } catch (e) {
        console.warn("Failed to initialize:", e);
        if (cancelled) {
          return;
        }
        await revealMainWindowForPermissions();
        setOnboardingState({ kind: "permissions", returning: true });
      }
    };

    void initializeAfterOnboarding();

    return () => {
      cancelled = true;
    };
  }, [onboardingState, refreshAudioDevices, refreshOutputDevices]);

  useEffect(() => {
    const handleKeyDown = (event: KeyboardEvent) => {
      const isDebugShortcut =
        event.shiftKey &&
        event.key.toLowerCase() === "d" &&
        (event.ctrlKey || event.metaKey);

      if (isDebugShortcut) {
        event.preventDefault();
        const currentDebugMode = settings?.debug_mode ?? false;
        updateSetting("debug_mode", !currentDebugMode);
      }
    };

    document.addEventListener("keydown", handleKeyDown);
    return () => {
      document.removeEventListener("keydown", handleKeyDown);
    };
  }, [settings?.debug_mode, updateSetting]);

  useEffect(() => {
    const unlisten = listen<RecordingErrorEvent>("recording-error", (event) => {
      const { error_type, detail } = event.payload;

      if (error_type === "microphone_permission_denied") {
        toast.error(t("errors.micPermissionDeniedTitle"), {
          description: t("errors.micPermissionDenied.macos"),
        });
      } else if (error_type === "no_input_device") {
        toast.error(t("errors.noInputDeviceTitle"), {
          description: t("errors.noInputDevice"),
        });
      } else {
        toast.error(
          t("errors.recordingFailed", { error: detail ?? "Unknown error" }),
        );
      }
    });
    return () => {
      unlisten.then((fn) => fn());
    };
  }, [t]);

  useEffect(() => {
    const unlisten = listen("paste-error", () => {
      toast.error(t("errors.pasteFailedTitle"), {
        description: t("errors.pasteFailed"),
      });
    });
    return () => {
      unlisten.then((fn) => fn());
    };
  }, [t]);

  useEffect(() => {
    const unlisten = listen<TranscriptionErrorEvent>(
      "transcription-error",
      (event) => {
        toast.error(t("errors.transcriptionFailedTitle"), {
          description: t(FAILURE_MESSAGE_KEYS[event.payload.kind]),
          action: {
            label: t("transcriptionFailedAction"),
            onClick: () => setCurrentSection("transcription"),
          },
        });
      },
    );
    return () => {
      unlisten.then((fn) => fn());
    };
  }, [t]);

  useEffect(() => {
    const unlisten = events.switchedToLocal.listen(() => {
      toast.success(t("localSttCard.switched"), {
        description: t("localSttCard.switchedDescription", {
          section: t("sidebar.transcription"),
        }),
      });
    });
    return () => {
      unlisten.then((fn) => fn());
    };
  }, [t]);

  const checkOnboardingStatus = async () => {
    try {
      const settingsResult = await commands.getAppSettings();
      const hasCompletedOnboarding =
        settingsResult.status === "ok" &&
        settingsResult.data.onboarding_completed === true;
      if (hasCompletedOnboarding) {
        try {
          const [hasAccessibility, hasMicrophone] = await Promise.all([
            checkAccessibilityPermission(),
            checkMicrophonePermission(),
          ]);
          if (!hasAccessibility || !hasMicrophone) {
            await revealMainWindowForPermissions();
            setOnboardingState({ kind: "permissions", returning: true });
            return;
          }
        } catch (e) {
          console.warn("Failed to check macOS permissions:", e);
        }

        try {
          await initializeKeyboardAutomation();
        } catch (e) {
          console.warn("Failed to initialize:", e);
          await revealMainWindowForPermissions();
          setOnboardingState({ kind: "permissions", returning: true });
          return;
        }

        hasCompletedPostOnboardingInit.current = true;
        refreshAudioDevices();
        refreshOutputDevices();
        setOnboardingState({ kind: "done" });
        return;
      }
    } catch (error) {
      console.error("Failed to check onboarding status:", error);
    }
    await startNewInstall();
  };

  /** Offers the local model first, unless this Mac cannot run it. */
  const startNewInstall = async () => {
    try {
      const result = await commands.getLocalModelStatus();
      if (result.status === "ok" && result.data.state.state !== "unsupported") {
        setOnboardingState({ kind: "model", status: result.data });
        return;
      }
    } catch (error) {
      console.warn("Failed to read the local model status:", error);
    }
    await saveEngine("cloud");
    setOnboardingState({
      kind: "permissions",
      returning: false,
      engine: "cloud",
    });
  };

  /** Picking this Mac also starts the model's download. */
  const saveEngine = async (engine: Engine): Promise<boolean> => {
    try {
      const result = await commands.changeTranscriptionEngineSetting(engine);
      if (result.status === "error") {
        console.warn("Failed to save the engine:", result.error);
        return false;
      }
      await refreshSettings();
      return true;
    } catch (error) {
      console.warn("Failed to save the engine:", error);
      return false;
    }
  };

  const handleModelChoice = async (engine: Engine) => {
    if (!(await saveEngine(engine))) {
      toast.error(t("settings.transcription.engine.error"));
      return;
    }
    setOnboardingState({ kind: "permissions", returning: false, engine });
  };

  const handlePermissionsComplete = useCallback(() => {
    setOnboardingState((current) => {
      if (current?.kind !== "permissions") return current;
      return current.returning
        ? { kind: "done" }
        : { kind: "account", engine: current.engine };
    });
  }, []);

  const handleProviderComplete = async (
    connected: boolean,
  ): Promise<boolean> => {
    try {
      const result = await commands.completeOnboarding();
      if (result.status === "error") {
        console.warn("Failed to complete onboarding:", result.error);
        toast.error(t("onboarding.provider.completionFailed"));
        return false;
      }
      await refreshSettings();
    } catch (e) {
      console.warn("Failed to complete onboarding:", e);
      toast.error(t("onboarding.provider.completionFailed"));
      return false;
    }
    // Onboarding keeps the cloud on a Mac that cannot run the local model.
    const saved = useSettingsStore.getState().settings;
    setOnboardingState(
      stepAfterAccount(
        saved?.transcription_engine,
        connected && (saved?.ai_cleanup_enabled ?? true),
      ),
    );
    return true;
  };

  const leaveStyleStep = () =>
    setOnboardingState((current) =>
      current?.kind === "style" ? { kind: current.next } : current,
    );

  const handleStyleChoice = async (style: DictationStyle): Promise<boolean> => {
    if (!(await updateSetting("dictation_style", style))) {
      toast.error(t("settings.transcription.style.error"));
      return false;
    }
    leaveStyleStep();
    return true;
  };

  /** From the finish step, after the download stopped. */
  const handleUseCloud = async () => {
    if (!(await saveEngine("cloud"))) {
      toast.error(t("settings.transcription.engine.error"));
      return;
    }
    const auth = await commands.getCodexAuthStatus().catch(() => null);
    setOnboardingState(
      auth?.signed_in ? { kind: "done" } : { kind: "account", engine: "cloud" },
    );
  };

  const toaster = (
    <Toaster
      theme="system"
      toastOptions={{
        unstyled: true,
        classNames: {
          toast:
            "bg-background border border-mid-gray/20 rounded-lg shadow-lg px-4 py-3 flex items-center gap-3 text-sm",
          title: "font-medium",
          description: "text-mid-gray",
          actionButton:
            "px-2 py-1 text-xs font-medium rounded-lg border text-on-accent bg-background-ui border-background-ui hover:bg-background-ui-hover hover:border-background-ui-hover cursor-pointer whitespace-nowrap",
        },
      }}
    />
  );

  if (onboardingState === null) {
    return null;
  }

  // While the user grants permissions and connects ChatGPT, the model
  // downloads; this says how far it got.
  const downloadingPill = settings?.transcription_engine === "local" && (
    <LocalModelPill
      status={localModel}
      className="pointer-events-none fixed top-3 end-3 z-10"
    />
  );

  let content: ReactNode;
  if (onboardingState.kind === "model") {
    content = (
      <LocalModelOnboarding
        status={onboardingState.status}
        onChoose={handleModelChoice}
      />
    );
  } else if (onboardingState.kind === "permissions") {
    content = (
      <>
        {!onboardingState.returning && downloadingPill}
        <AccessibilityOnboarding onComplete={handlePermissionsComplete} />
      </>
    );
  } else if (onboardingState.kind === "account") {
    content = (
      <>
        {downloadingPill}
        <ProviderOnboarding
          onComplete={handleProviderComplete}
          optional={onboardingState.engine === "local"}
        />
      </>
    );
  } else if (onboardingState.kind === "style") {
    content = (
      <>
        {downloadingPill}
        <StyleOnboarding
          stored={settings?.dictation_style ?? "normal"}
          onContinue={handleStyleChoice}
          onSkip={leaveStyleStep}
        />
      </>
    );
  } else if (onboardingState.kind === "finish") {
    content = (
      <DoneOnboarding
        onFinish={() => setOnboardingState({ kind: "done" })}
        onUseCloud={handleUseCloud}
      />
    );
  } else {
    content = (
      <div className="h-screen flex flex-col select-none cursor-default">
        <ErrorBoundary context="What's New">
          <WhatsNewGate />
        </ErrorBoundary>
        <div className="flex-1 flex overflow-hidden">
          <Sidebar
            activeSection={currentSection}
            onSectionChange={setCurrentSection}
          />
          <div className="flex-1 flex flex-col overflow-hidden">
            <div className="flex-1 overflow-y-auto">
              <div className="flex flex-col items-center p-4 gap-4">
                {settings?.transcription_engine === "local" && (
                  <LocalModelPill
                    status={localModel}
                    className="flex w-full max-w-3xl justify-end"
                  />
                )}
                <AccessibilityPermissions />
                {(currentSection === "general" ||
                  currentSection === "transcription") && <LocalSttCard />}
                {renderSettingsContent(currentSection)}
              </div>
            </div>
          </div>
        </div>
        <Footer />
      </div>
    );
  }

  return (
    <>
      {toaster}
      {content}
    </>
  );
}

export default App;
