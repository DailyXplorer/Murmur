import { useCallback, useEffect, useState } from "react";
import { openUrl } from "@tauri-apps/plugin-opener";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import {
  type ProviderStatus,
  statusTranslationKey,
  useCodexStatus,
} from "@/components/transcription/providerStatus";
import { Button } from "../ui/Button";
import MurmurTextLogo from "../icons/MurmurTextLogo";

interface ProviderOnboardingProps {
  /** `connected`: the step ends with Continue, signed in, not with Skip. */
  onComplete: (connected: boolean) => Promise<boolean>;
  /** Dictation runs on this Mac, so the account only adds AI cleanup and
   * more languages, and the step can be skipped. */
  optional: boolean;
}

const ProviderOnboarding: React.FC<ProviderOnboardingProps> = ({
  onComplete,
  optional,
}) => {
  const { t } = useTranslation();
  const { invalidatePendingRefreshes, refreshStatus, status } =
    useCodexStatus();
  const [isCompleting, setIsCompleting] = useState(false);
  const [retried, setRetried] = useState(false);
  // The last finished check, so a refresh on focus does not swap Continue
  // for Start while it runs.
  const [checked, setChecked] = useState<ProviderStatus | null>(null);
  if (status.kind !== "checking" && status !== checked) setChecked(status);

  useEffect(() => {
    void refreshStatus();
    const handleFocus = () => void refreshStatus();
    window.addEventListener("focus", handleFocus);
    return () => {
      invalidatePendingRefreshes();
      window.removeEventListener("focus", handleFocus);
    };
  }, [invalidatePendingRefreshes, refreshStatus]);

  const openCodexSetup = useCallback(async () => {
    try {
      await openUrl("https://openai.com/codex/");
    } catch (error) {
      console.warn("Failed to open Codex setup:", error);
      toast.error(t("onboarding.provider.codexSetupFailed"));
    }
  }, [t]);

  const connected = checked?.kind === "configured";
  const canComplete = !isCompleting && connected;
  const canSkip = optional && !isCompleting && !connected;
  // "Not signed in" is what the description already says, so it shows only
  // once the user asked to check again.
  const line =
    retried && status.kind === "checking"
      ? status
      : checked?.kind === "error" ||
          (checked?.kind === "unavailable" &&
            (retried || checked.reason === "sessionExpired"))
        ? checked
        : null;

  const finish = useCallback(async () => {
    setIsCompleting(true);
    try {
      const completed = await onComplete(connected);
      if (!completed) {
        await refreshStatus();
      }
    } finally {
      setIsCompleting(false);
    }
  }, [connected, onComplete, refreshStatus]);

  const complete = useCallback(async () => {
    if (canComplete) await finish();
  }, [canComplete, finish]);

  const skip = useCallback(async () => {
    if (canSkip) await finish();
  }, [canSkip, finish]);

  return (
    <div
      data-testid="provider-onboarding"
      className="h-[100dvh] min-h-screen w-screen overflow-y-auto p-3 sm:p-6"
    >
      <div className="mx-auto flex min-h-full w-full max-w-xl flex-col items-center justify-center gap-2">
        <MurmurTextLogo width={160} />
        <div className="w-full max-w-xl">
          <div className="mb-2 text-center">
            <h2 className="font-title text-2xl font-normal text-text">
              {optional
                ? t("onboarding.provider.optionalTitle")
                : t("onboarding.provider.title")}
            </h2>
            <p className="mt-1 text-pretty text-sm text-text/70">
              {optional
                ? t("onboarding.provider.optionalDescription")
                : t("onboarding.provider.description")}
            </p>
          </div>
          <div className="mt-3 flex justify-center">
            {/* Hidden until the first check answers, so a connected user
                never sees Start. */}
            <div
              className={`inline-grid grid-flow-col auto-cols-fr gap-2 ${checked ? "" : "invisible"}`}
            >
              {connected ? (
                <Button onClick={() => void complete()}>
                  {t("onboarding.provider.continue")}
                </Button>
              ) : (
                <Button onClick={() => void openCodexSetup()}>
                  {t("onboarding.provider.start")}
                </Button>
              )}
              <Button
                onClick={() => {
                  setRetried(true);
                  void refreshStatus();
                }}
                disabled={isCompleting}
              >
                {t("onboarding.provider.retry")}
              </Button>
              {optional && !connected && (
                <Button onClick={() => void skip()} disabled={!canSkip}>
                  {t("onboarding.provider.skip")}
                </Button>
              )}
            </div>
          </div>
          <p
            role="status"
            className="mt-2 min-h-5 text-center text-sm text-text/70"
          >
            {line && t(statusTranslationKey(line))}
          </p>
        </div>
      </div>
    </div>
  );
};

export default ProviderOnboarding;
