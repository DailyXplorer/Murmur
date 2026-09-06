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
import { getLanguageDirection, initializeRTL } from "@/lib/utils/rtl";

type OnboardingState =
  | { kind: "permissions"; configuredProvider: TranscriptionProvider }
  | { kind: "provider"; configuredProvider: TranscriptionProvider }
  | { kind: "done" };

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
          configuredProvider: "codex",
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
    const unlisten = listen<string>("transcription-error", (event) => {
      toast.error(t("errors.transcriptionFailedTitle"), {
        description: event.payload,
        action: {
          label: t("transcriptionFailedAction"),
          onClick: () => setCurrentSection("transcription"),
        },
      });
    });
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

  /** Persists the user's explicit choice before asking the backend to complete setup. */
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
        toast.error(t("onboarding.provider.completionFailed"), {
          description: result.error,
        });
        return false;
      }
      await refreshSettings();
    } catch (e) {
      console.warn("Failed to complete onboarding:", e);
      toast.error(t("onboarding.provider.completionFailed"), {
        description: e instanceof Error ? e.message : String(e),
      });
      return false;
    }
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
            "px-2 py-1 text-xs font-medium rounded-lg border bg-mid-gray/10 border-mid-gray/20 hover:bg-background-ui/30 hover:border-logo-primary cursor-pointer whitespace-nowrap",
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
                <AccessibilityPermissions />
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
