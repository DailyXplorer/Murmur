import { useEffect, useState, useCallback, useRef } from "react";
import { useTranslation } from "react-i18next";
import {
  checkAccessibilityPermission,
  checkMicrophonePermission,
  requestMicrophonePermission,
} from "tauri-plugin-macos-permissions-api";
import { toast } from "sonner";
import { commands } from "@/bindings";
import { repairAccessibilityPermission } from "@/lib/macosPermissions";
import { useSettingsStore } from "@/stores/settingsStore";
import MurmurTextLogo from "../icons/MurmurTextLogo";
import { CheckIcon } from "@phosphor-icons/react/dist/csr/Check";
import { CircleNotchIcon } from "@phosphor-icons/react/dist/csr/CircleNotch";
import { KeyboardIcon } from "@phosphor-icons/react/dist/csr/Keyboard";
import { MicrophoneIcon } from "@phosphor-icons/react/dist/csr/Microphone";

interface AccessibilityOnboardingProps {
  onComplete: () => void;
}

type PermissionStatus = "checking" | "needed" | "waiting" | "granted";

interface PermissionsState {
  accessibility: PermissionStatus;
  microphone: PermissionStatus;
}

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

const AccessibilityOnboarding: React.FC<AccessibilityOnboardingProps> = ({
  onComplete,
}) => {
  const { t } = useTranslation();
  const refreshAudioDevices = useSettingsStore(
    (state) => state.refreshAudioDevices,
  );
  const refreshOutputDevices = useSettingsStore(
    (state) => state.refreshOutputDevices,
  );
  const [permissions, setPermissions] = useState<PermissionsState>({
    accessibility: "checking",
    microphone: "checking",
  });
  const [isRepairingAccessibility, setIsRepairingAccessibility] =
    useState(false);
  const pollingRef = useRef<ReturnType<typeof setInterval> | null>(null);
  const timeoutRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const errorCountRef = useRef<number>(0);
  const MAX_POLLING_ERRORS = 3;

  const allGranted =
    permissions.accessibility === "granted" &&
    permissions.microphone === "granted";

  const completeOnboarding = useCallback(async () => {
    await Promise.all([refreshAudioDevices(), refreshOutputDevices()]);
    timeoutRef.current = setTimeout(() => onComplete(), 300);
  }, [onComplete, refreshAudioDevices, refreshOutputDevices]);

  useEffect(() => {
    const checkInitial = async () => {
      try {
        const [accessibilityGranted, microphoneGranted] = await Promise.all([
          checkAccessibilityPermission(),
          checkMicrophonePermission(),
        ]);

        if (accessibilityGranted) {
          await initializeKeyboardAutomation();
        }

        const newState: PermissionsState = {
          accessibility: accessibilityGranted ? "granted" : "needed",
          microphone: microphoneGranted ? "granted" : "needed",
        };

        setPermissions(newState);

        if (accessibilityGranted && microphoneGranted) {
          await completeOnboarding();
        }
      } catch (error) {
        console.error("Failed to check macOS permissions:", error);
        toast.error(t("onboarding.permissions.errors.checkFailed"));
        setPermissions({
          accessibility: "needed",
          microphone: "needed",
        });
      }
    };

    checkInitial();
  }, [completeOnboarding, t]);

  const startPolling = useCallback(() => {
    if (pollingRef.current) return;

    pollingRef.current = setInterval(async () => {
      try {
        const [accessibilityGranted, microphoneGranted] = await Promise.all([
          checkAccessibilityPermission(),
          checkMicrophonePermission(),
        ]);

        if (accessibilityGranted) {
          await initializeKeyboardAutomation();
        }

        setPermissions((prev) => {
          const newState = { ...prev };

          if (accessibilityGranted && prev.accessibility !== "granted") {
            newState.accessibility = "granted";
          }

          if (microphoneGranted && prev.microphone !== "granted") {
            newState.microphone = "granted";
          }

          return newState;
        });

        if (accessibilityGranted && microphoneGranted) {
          if (pollingRef.current) {
            clearInterval(pollingRef.current);
            pollingRef.current = null;
          }
          await completeOnboarding();
        }

        errorCountRef.current = 0;
      } catch (error) {
        console.error("Error checking permissions:", error);
        errorCountRef.current += 1;

        if (errorCountRef.current >= MAX_POLLING_ERRORS) {
          if (pollingRef.current) {
            clearInterval(pollingRef.current);
            pollingRef.current = null;
          }
          toast.error(t("onboarding.permissions.errors.checkFailed"));
          setPermissions({
            accessibility: "needed",
            microphone: "needed",
          });
        }
      }
    }, 1000);
  }, [completeOnboarding, t]);

  useEffect(() => {
    return () => {
      if (pollingRef.current) {
        clearInterval(pollingRef.current);
      }
      if (timeoutRef.current) {
        clearTimeout(timeoutRef.current);
      }
    };
  }, []);

  const handleGrantAccessibility = async () => {
    if (isRepairingAccessibility) return;

    setIsRepairingAccessibility(true);
    setPermissions((prev) => ({ ...prev, accessibility: "waiting" }));

    try {
      await repairAccessibilityPermission();
      startPolling();
    } catch (error) {
      console.error("Failed to repair accessibility permission:", error);
      setPermissions((prev) => ({ ...prev, accessibility: "needed" }));
      toast.error(t("onboarding.permissions.errors.requestFailed"));
    } finally {
      setIsRepairingAccessibility(false);
    }
  };

  const handleGrantMicrophone = async () => {
    try {
      await requestMicrophonePermission();
      setPermissions((prev) => ({ ...prev, microphone: "waiting" }));
      startPolling();
    } catch (error) {
      console.error("Failed to request microphone permission:", error);
      toast.error(t("onboarding.permissions.errors.requestFailed"));
    }
  };

  const isChecking =
    permissions.accessibility === "checking" &&
    permissions.microphone === "checking";

  if (isChecking) {
    return (
      <div className="h-screen w-screen flex items-center justify-center">
        <CircleNotchIcon size={28} className="animate-spin text-text/50" />
      </div>
    );
  }

  if (allGranted) {
    return (
      <div className="h-screen w-screen flex flex-col items-center justify-center gap-4">
        <div className="p-4 rounded-full bg-emerald-500/20">
          <CheckIcon size={40} className="text-emerald-400" />
        </div>
        <p className="text-lg font-medium text-text">
          {t("onboarding.permissions.allGranted")}
        </p>
      </div>
    );
  }

  return (
    <div className="h-screen w-screen flex flex-col p-6 gap-6 items-center justify-center">
      <div className="flex flex-col items-center gap-2">
        <MurmurTextLogo width={200} />
      </div>

      <div className="max-w-md w-full flex flex-col items-center gap-4">
        <div className="text-center mb-2">
          <h2 className="text-xl font-semibold text-text mb-2 text-balance">
            {t("onboarding.permissions.title")}
          </h2>
          <p className="text-text/70 text-pretty">
            {t("onboarding.permissions.description")}
          </p>
        </div>

        <div className="w-full p-4 rounded-lg bg-white/5 border border-mid-gray/20">
          <div className="flex items-center gap-4">
            <div className="p-3 rounded-full bg-logo-primary/20 shrink-0">
              <MicrophoneIcon size={22} className="text-logo-primary" />
            </div>
            <div className="flex-1 min-w-0">
              <h3 className="font-medium text-text">
                {t("onboarding.permissions.microphone.title")}
              </h3>
              <p className="text-sm text-text/60 mb-3 text-pretty">
                {t("onboarding.permissions.microphone.description")}
              </p>
              {permissions.microphone === "granted" ? (
                <div className="flex items-center gap-2 text-emerald-400 text-sm">
                  <CheckIcon size={15} />
                  {t("onboarding.permissions.granted")}
                </div>
              ) : permissions.microphone === "waiting" ? (
                <div className="flex items-center gap-2 text-text/50 text-sm">
                  <CircleNotchIcon size={15} className="animate-spin" />
                  {t("onboarding.permissions.waiting")}
                </div>
              ) : (
                <button
                  onClick={handleGrantMicrophone}
                  className="min-h-10 px-4 py-2 rounded-lg bg-background-ui hover:bg-background-ui-hover text-on-accent text-sm font-medium active:scale-[0.96] transition-[background-color,transform]"
                >
                  {t("onboarding.permissions.grant")}
                </button>
              )}
            </div>
          </div>
        </div>

        <div className="w-full p-4 rounded-lg bg-white/5 border border-mid-gray/20">
          <div className="flex items-center gap-4">
            <div className="p-3 rounded-full bg-logo-primary/20 shrink-0">
              <KeyboardIcon size={22} className="text-logo-primary" />
            </div>
            <div className="flex-1 min-w-0">
              <h3 className="font-medium text-text">
                {t("onboarding.permissions.accessibility.title")}
              </h3>
              <p className="text-sm text-text/60 mb-3 text-pretty">
                {t("onboarding.permissions.accessibility.description")}
              </p>
              {permissions.accessibility === "granted" ? (
                <div className="flex items-center gap-2 text-emerald-400 text-sm">
                  <CheckIcon size={15} />
                  {t("onboarding.permissions.granted")}
                </div>
              ) : permissions.accessibility === "waiting" ? (
                <div
                  aria-live="polite"
                  className="flex flex-wrap items-center gap-2 rounded-lg bg-logo-primary/10 px-3 py-2 text-sm text-text"
                >
                  <span className="flex min-h-10 items-center gap-2">
                    <CircleNotchIcon size={15} className="animate-spin" />
                    {t("onboarding.permissions.accessibility.waiting")}
                  </span>
                  <button
                    type="button"
                    disabled={isRepairingAccessibility}
                    onClick={handleGrantAccessibility}
                    className="min-h-10 rounded-md px-2 font-medium underline underline-offset-2 hover:text-logo-primary disabled:cursor-wait disabled:opacity-50 transition-colors"
                  >
                    {t("accessibility.openSettings")}
                  </button>
                </div>
              ) : (
                <button
                  onClick={handleGrantAccessibility}
                  className="min-h-10 px-4 py-2 rounded-lg bg-background-ui hover:bg-background-ui-hover text-on-accent text-sm font-medium active:scale-[0.96] transition-[background-color,transform]"
                >
                  {t("onboarding.permissions.accessibility.action")}
                </button>
              )}
            </div>
          </div>
        </div>
      </div>
    </div>
  );
};

export default AccessibilityOnboarding;
