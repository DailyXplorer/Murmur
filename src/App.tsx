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
  ProviderOnboarding,
} from "./components/onboarding";
import { ErrorBoundary } from "./components/ErrorBoundary";
import { Sidebar, SidebarSection, SECTIONS_CONFIG } from "./components/Sidebar";
import { WhatsNewGate } from "./components/whats-new";
import { useSettings } from "./hooks/useSettings";
import { useSettingsStore } from "./stores/settingsStore";
import { commands, type TranscriptionProvider } from "@/bindings";
import { isWindows } from "@/lib/platform";
import { getLanguageDirection, initializeRTL } from "@/lib/utils/rtl";

type OnboardingState =
  | { kind: "permissions"; configuredProvider: TranscriptionProvider }
  | { kind: "provider"; configuredProvider: TranscriptionProvider }
  | { kind: "done" };

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
  | "other";

interface TranscriptionErrorEvent {
  kind: FailureKind;
  detail: string;
  provider: TranscriptionProvider;
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
  other: "tray.errorTranscription",
};

/** Gemini runs on the Antigravity session, not on Codex's. */
const failureMessageKey = ({ kind, provider }: TranscriptionErrorEvent) =>
  provider === "gemini" && (kind === "session_expired" || kind === "signed_out")
    ? "tray.errorGeminiSessionExpired"
    : FAILURE_MESSAGE_KEYS[kind];

const transcriptionProviderFromSettings = (
  provider: TranscriptionProvider | undefined,
): TranscriptionProvider => (provider === "gemini" ? "gemini" : "codex");

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

/** Windows asks for no Accessibility permission, only the microphone. */
const hasRequiredPermissions = async (): Promise<boolean> => {
  if (isWindows) {
    const result = await commands.getMicrophoneAuthorizationStatus();
    if (result.status === "error") throw new Error(result.error);
    return result.data === 3;
  }
  const [hasAccessibility, hasMicrophone] = await Promise.all([
    checkAccessibilityPermission(),
    checkMicrophonePermission(),
  ]);
  return hasAccessibility && hasMicrophone;
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
  const { t, i18n } = useTranslation();
  const [onboardingState, setOnboardingState] =
    useState<OnboardingState | null>(null);
  const [currentSection, setCurrentSection] =
    useState<SidebarSection>("general");
  const { settings, updateSetting, refreshSettings } = useSettings();
  const direction = getLanguageDirection(i18n.language);
  const refreshAudioDevices = useSettingsStore(
    (state) => state.refreshAudioDevices,
  );
  const refreshOutputDevices = useSettingsStore(
    (state) => state.refreshOutputDevices,
  );
  const hasCompletedPostOnboardingInit = useRef(false);
  const configuredProviderRef = useRef<TranscriptionProvider>("codex");

  useEffect(() => {
    checkOnboardingStatus();
  }, []);

  useEffect(() => {
    initializeRTL(i18n.language);
  }, [i18n.language]);

  useEffect(() => {
    if (
      onboardingState?.kind !== "done" ||
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
        setOnboardingState({
          kind: "permissions",
          configuredProvider: configuredProviderRef.current,
        });
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
          description: t(
            isWindows
              ? "errors.micPermissionDenied.windows"
              : "errors.micPermissionDenied.macos",
          ),
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
          description: t(failureMessageKey(event.payload)),
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

  const checkOnboardingStatus = async () => {
    try {
      const settingsResult = await commands.getAppSettings();
      const configuredProvider = transcriptionProviderFromSettings(
        settingsResult.status === "ok"
          ? settingsResult.data.transcription_provider
          : undefined,
      );
      configuredProviderRef.current = configuredProvider;
      const hasCompletedOnboarding =
        settingsResult.status === "ok" &&
        settingsResult.data.onboarding_completed === true;
      if (hasCompletedOnboarding) {
        try {
          if (!(await hasRequiredPermissions())) {
            await revealMainWindowForPermissions();
            setOnboardingState({ kind: "permissions", configuredProvider });
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
          setOnboardingState({ kind: "permissions", configuredProvider });
          return;
        }

        hasCompletedPostOnboardingInit.current = true;
        refreshAudioDevices();
        refreshOutputDevices();
        setOnboardingState({ kind: "done" });
      } else {
        setOnboardingState({ kind: "permissions", configuredProvider });
      }
    } catch (error) {
      console.error("Failed to check onboarding status:", error);
      setOnboardingState({
        kind: "permissions",
        configuredProvider: "codex",
      });
    }
  };

  const handlePermissionsComplete = useCallback(() => {
    setOnboardingState((current) => {
      if (current?.kind !== "permissions") return current;
      return {
        kind: "provider",
        configuredProvider: current.configuredProvider,
      };
    });
  }, []);

  const handleProviderComplete = async (
    provider: TranscriptionProvider,
  ): Promise<boolean> => {
    try {
      const persisted = await updateSetting("transcription_provider", provider);
      if (!persisted) {
        toast.error(t("onboarding.provider.providerChangeFailed"));
        return false;
      }
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
    configuredProviderRef.current = provider;
    setOnboardingState({ kind: "done" });
    return true;
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

  let content: ReactNode;
  if (onboardingState.kind === "permissions") {
    content = (
      <AccessibilityOnboarding onComplete={handlePermissionsComplete} />
    );
  } else if (onboardingState.kind === "provider") {
    content = (
      <ProviderOnboarding
        configuredProvider={onboardingState.configuredProvider}
        onComplete={handleProviderComplete}
      />
    );
  } else {
    content = (
      <div
        dir={direction}
        className="h-screen flex flex-col select-none cursor-default"
      >
        {/* The Windows beta's notes live on its GitHub pre-release. */}
        {!isWindows && (
          <ErrorBoundary context="What's New">
            <WhatsNewGate />
          </ErrorBoundary>
        )}
        <div className="flex-1 flex overflow-hidden">
          <Sidebar
            activeSection={currentSection}
            onSectionChange={setCurrentSection}
          />
          <div className="flex-1 flex flex-col overflow-hidden">
            <div className="flex-1 overflow-y-auto">
              <div className="flex flex-col items-center p-4 gap-4">
                {!isWindows && <AccessibilityPermissions />}
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
