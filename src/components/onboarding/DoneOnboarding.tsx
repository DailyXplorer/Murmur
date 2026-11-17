import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { commands, events } from "@/bindings";
import { useCodexStatus } from "@/components/transcription/providerStatus";
import { downloadPercent, useLocalModel } from "@/hooks/useLocalModel";
import { useKeyNames } from "@/hooks/useKeyNames";
import { useSettings } from "@/hooks/useSettings";
import { formatKeyCombination } from "@/lib/utils/keyboard";
import { Button } from "../ui/Button";
import MurmurTextLogo from "../icons/MurmurTextLogo";
import {
  failureText,
  useModelAction,
} from "../settings/transcription/LocalModelRow";

interface DoneOnboardingProps {
  onFinish: () => void;
  /** Leaves the local model for the cloud. */
  onUseCloud: () => Promise<void>;
}

/** The last step when dictation runs on this Mac: says whether the model
 * is there yet and what happens until it is, and lets the user try. Murmur
 * is the app in front here, so a dictation cannot be pasted: the backend
 * hands it to this page instead (`PracticeDictation`), which shows it. */
const DoneOnboarding: React.FC<DoneOnboardingProps> = ({
  onFinish,
  onUseCloud,
}) => {
  const { t, i18n } = useTranslation();
  const model = useLocalModel();
  const { refreshStatus, status: account } = useCodexStatus();
  const run = useModelAction("settings.transcription.localModel.error");
  const [leaving, setLeaving] = useState(false);
  const { getSetting } = useSettings();
  const keyNames = useKeyNames();
  const [practiced, setPracticed] = useState<string | null>(null);

  useEffect(() => {
    void refreshStatus();
  }, [refreshStatus]);

  useEffect(() => {
    let disposed = false;
    const listening = events.practiceDictation.listen((event) => {
      if (!disposed) setPracticed(event.payload);
    });
    return () => {
      disposed = true;
      void listening.then((stop) => stop());
    };
  }, []);

  if (!model) return null;

  const state = model.state;
  const stopped = state.state === "failed" || state.state === "missing";
  const meanwhile =
    account.kind === "configured"
      ? t("onboarding.done.cloudMeanwhile")
      : account.kind === "checking"
        ? null
        : t("onboarding.done.waitForIt");

  // What is said goes somewhere now: on this Mac, or in the cloud meanwhile.
  const canDictate =
    state.state === "ready" ||
    state.state === "optimizing" ||
    state.state === "unsupported" ||
    ((state.state === "downloading" || state.state === "preparing") &&
      account.kind === "configured");
  const shortcut = formatKeyCombination(
    getSetting("bindings")?.transcribe?.current_binding ?? "",
    {},
    keyNames,
  );

  const text = (() => {
    // A download that stopped meanwhile still has to be said.
    if (practiced !== null && !stopped) return t("onboarding.done.practiced");
    switch (state.state) {
      case "ready":
      case "optimizing":
      case "unsupported":
        return shortcut
          ? t("onboarding.done.readyWith", { shortcut })
          : t("onboarding.done.ready");
      case "downloading":
        return [
          t("onboarding.done.downloading", { percent: downloadPercent(model) }),
          meanwhile,
        ]
          .filter(Boolean)
          .join(" ");
      case "preparing":
        return [t("onboarding.done.preparing"), meanwhile]
          .filter(Boolean)
          .join(" ");
      case "failed":
        return failureText(t, state.reason, i18n.language);
      case "missing":
        return t("settings.transcription.localModel.missing");
      default: {
        const _exhaustive: never = state;
        return _exhaustive;
      }
    }
  })();

  const switchToCloud = async () => {
    setLeaving(true);
    try {
      await onUseCloud();
    } finally {
      setLeaving(false);
    }
  };

  return (
    <div
      data-testid="done-onboarding"
      className="h-[100dvh] min-h-screen w-screen overflow-y-auto p-3 sm:p-6"
    >
      <div className="mx-auto flex min-h-full w-full max-w-md flex-col items-center justify-center gap-4">
        <MurmurTextLogo width={160} />
        <div className="text-center" aria-live="polite">
          <h2 className="text-balance font-title text-2xl font-normal text-text">
            {stopped
              ? t("onboarding.done.stoppedTitle")
              : t("onboarding.done.title")}
          </h2>
          <p className="mt-2 text-pretty text-text/70">{text}</p>
        </div>
        {/* What was said stays, even if nothing more can be dictated. */}
        {(canDictate || practiced !== null) && (
          <p
            role="status"
            aria-label={t("onboarding.done.practiceLabel")}
            data-testid="practice"
            className={`min-h-24 w-full rounded-lg border px-4 py-3 text-start text-pretty transition-colors duration-150 ${
              practiced === null
                ? "border-dashed border-mid-gray/40 text-text/50"
                : "border-logo-primary/60 bg-logo-primary/5 text-text"
            }`}
          >
            {practiced ?? t("onboarding.done.practiceHint")}
          </p>
        )}
        <div className="flex flex-col items-center gap-2">
          <div className="flex flex-wrap justify-center gap-2">
            {stopped && (
              <Button
                onClick={(event) =>
                  void run(event, commands.downloadLocalModel)
                }
              >
                {t("settings.transcription.localModel.retry")}
              </Button>
            )}
            <Button onClick={onFinish}>{t("onboarding.done.start")}</Button>
          </div>
          {stopped && (
            <button
              type="button"
              disabled={leaving}
              onClick={() => void switchToCloud()}
              className="min-h-control rounded-md px-2 text-sm font-medium text-text/70 underline underline-offset-2 hover:text-accent-text disabled:cursor-wait disabled:opacity-50 transition-colors"
            >
              {t("onboarding.model.useCloud")}
            </button>
          )}
        </div>
      </div>
    </div>
  );
};

export default DoneOnboarding;
