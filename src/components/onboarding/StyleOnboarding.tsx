import { useState } from "react";
import { useTranslation } from "react-i18next";
import type { DictationStyle } from "@/bindings";
import { Button } from "../ui/Button";
import { RadioOption } from "../ui/RadioOption";
import MurmurTextLogo from "../icons/MurmurTextLogo";
import { DICTATION_STYLES } from "../settings/DictationStyle";

interface StyleOnboardingProps {
  stored: DictationStyle;
  /** Saves the style and moves on; false keeps the step open. */
  onContinue: (style: DictationStyle) => Promise<boolean>;
  /** Moves on with the stored style. */
  onSkip: () => void;
}

/** Shown after the account step when AI cleanup can run. */
const StyleOnboarding: React.FC<StyleOnboardingProps> = ({
  stored,
  onContinue,
  onSkip,
}) => {
  const { t } = useTranslation();
  const [style, setStyle] = useState(stored);
  const [saving, setSaving] = useState(false);

  const save = async () => {
    setSaving(true);
    if (!(await onContinue(style))) setSaving(false);
  };

  return (
    <div
      data-testid="style-onboarding"
      className="h-[100dvh] min-h-screen w-screen overflow-y-auto p-3 sm:p-6"
    >
      <div className="mx-auto flex min-h-full w-full max-w-md flex-col items-center justify-center gap-4">
        <MurmurTextLogo width={160} />
        <div className="text-center">
          <h2 className="text-balance font-title text-2xl font-normal text-text">
            {t("onboarding.style.title")}
          </h2>
          <p className="mt-2 text-pretty text-text/70">
            {t("onboarding.style.description")}
          </p>
        </div>
        <div
          role="radiogroup"
          aria-label={t("settings.transcription.style.title")}
          className="w-full divide-y divide-mid-gray/20 rounded-lg border border-mid-gray/20 bg-background"
        >
          {DICTATION_STYLES.map((option) => (
            <RadioOption
              key={option}
              name="dictation-style"
              value={option}
              checked={style === option}
              disabled={saving}
              title={t(`settings.transcription.style.options.${option}`)}
              onSelect={setStyle}
            />
          ))}
        </div>
        <div className="inline-grid grid-flow-col auto-cols-fr gap-2">
          <Button disabled={saving} onClick={() => void save()}>
            {t("onboarding.provider.continue")}
          </Button>
          <Button disabled={saving} onClick={onSkip}>
            {t("onboarding.provider.skip")}
          </Button>
        </div>
      </div>
    </div>
  );
};

export default StyleOnboarding;
