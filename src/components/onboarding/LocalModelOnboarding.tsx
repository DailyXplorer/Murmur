import { useState } from "react";
import { useTranslation } from "react-i18next";
import type { Engine, LocalModelStatus } from "@/bindings";
import { formatSize } from "@/lib/meetings/format";
import { Button } from "../ui/Button";
import MurmurTextLogo from "../icons/MurmurTextLogo";

interface LocalModelOnboardingProps {
  status: LocalModelStatus;
  /** Saves the engine and moves on; the local model's download starts in
   * the background. */
  onChoose: (engine: Engine) => Promise<void>;
}

/** The first step of a new install: dictate on this Mac, the default, or in
 * the cloud. */
const LocalModelOnboarding: React.FC<LocalModelOnboardingProps> = ({
  status,
  onChoose,
}) => {
  const { t, i18n } = useTranslation();
  const [choosing, setChoosing] = useState(false);
  const ready = status.state.state === "ready";

  const choose = async (engine: Engine) => {
    if (choosing) return;
    setChoosing(true);
    try {
      await onChoose(engine);
    } finally {
      setChoosing(false);
    }
  };

  return (
    <div
      data-testid="local-model-onboarding"
      className="h-[100dvh] min-h-screen w-screen overflow-y-auto p-3 sm:p-6"
    >
      <div className="mx-auto flex min-h-full w-full max-w-md flex-col items-center justify-center gap-4">
        <MurmurTextLogo width={160} />
        <div className="text-center">
          <h2 className="text-balance font-title text-2xl font-normal text-text">
            {t("onboarding.model.title")}
          </h2>
          <p className="mt-2 text-pretty text-text/70">
            {t("onboarding.model.description")}{" "}
            {ready
              ? t("onboarding.model.alreadyHere")
              : t("onboarding.model.download", {
                  size: formatSize(status.size_bytes, i18n.language),
                })}
          </p>
          <p className="mt-2 text-pretty text-sm text-text/60">
            {t("onboarding.model.languages")}
          </p>
        </div>
        <div className="flex flex-col items-center gap-2">
          <Button disabled={choosing} onClick={() => void choose("local")}>
            {ready
              ? t("onboarding.provider.continue")
              : t("onboarding.model.downloadAndContinue")}
          </Button>
          <button
            type="button"
            disabled={choosing}
            onClick={() => void choose("cloud")}
            className="min-h-control rounded-md px-2 text-sm font-medium text-text/70 underline underline-offset-2 hover:text-accent-text disabled:cursor-wait disabled:opacity-50 transition-colors"
          >
            {t("onboarding.model.useCloud")}
          </button>
        </div>
      </div>
    </div>
  );
};

export default LocalModelOnboarding;
