import React, { useId } from "react";
import { useTranslation } from "react-i18next";
import { XIcon } from "@phosphor-icons/react/dist/csr/X";
import { commands } from "@/bindings";
import { downloadPercent, useLocalModel } from "@/hooks/useLocalModel";
import { useSettings } from "@/hooks/useSettings";
import { formatSize } from "@/lib/meetings/format";
import { Button } from "./ui/Button";
import { IconButton } from "./ui/IconButton";
import {
  failureText,
  useModelAction,
} from "./settings/transcription/LocalModelRow";

/** Tells existing cloud users once that dictation can run on this Mac. Its
 * button downloads the model; Murmur switches to it once it is ready. */
export const LocalSttCard: React.FC = () => {
  const { t, i18n } = useTranslation();
  const { settings, updateSetting, refreshSettings } = useSettings();
  const model = useLocalModel();
  const titleId = useId();
  const run = useModelAction("localSttCard.error");

  if (
    !settings ||
    !model ||
    settings.transcription_engine !== "cloud" ||
    settings.local_stt_card_dismissed ||
    model.state.state === "unsupported"
  ) {
    return null;
  }

  const switchToLocal = async (event: React.MouseEvent) => {
    await run(event, commands.switchToLocalWhenReady);
    await refreshSettings();
  };
  // A download started elsewhere, by Meetings, switches nothing yet.
  const switching = settings.switch_to_local_when_ready === true;
  const { state } = model;
  const switchLabel = t("localSttCard.useLocal");
  const [text, button] = ((): [string, string | null] => {
    switch (state.state) {
      case "missing":
        return [
          t("localSttCard.description"),
          t("settings.transcription.localModel.download", {
            size: formatSize(model.size_bytes, i18n.language),
          }),
        ];
      case "ready":
        return [t("localSttCard.description"), switchLabel];
      case "downloading":
        return switching
          ? [
              t("localSttCard.downloading", {
                percent: downloadPercent(model),
              }),
              null,
            ]
          : [t("localSttCard.description"), switchLabel];
      case "preparing":
        return switching
          ? [t("settings.transcription.localModel.preparing"), null]
          : [t("localSttCard.description"), switchLabel];
      case "failed":
        return [
          failureText(t, state.reason, i18n.language),
          t("settings.transcription.localModel.retry"),
        ];
      default: {
        const _exhaustive: never = state;
        return _exhaustive;
      }
    }
  })();

  return (
    <section
      aria-labelledby={titleId}
      className="mx-auto flex w-full max-w-3xl items-center gap-3 rounded-lg border border-mid-gray/20 px-4 py-3"
    >
      <div className="min-w-0 flex-1">
        <h2 id={titleId} className="text-sm font-medium">
          {t("localSttCard.title")}
        </h2>
        <p className="text-pretty text-sm text-text/70">{text}</p>
      </div>
      {button && (
        <Button onClick={(event) => void switchToLocal(event)}>{button}</Button>
      )}
      <IconButton
        size="compact"
        label={t("localSttCard.dismiss")}
        onClick={() => void updateSetting("local_stt_card_dismissed", true)}
      >
        <XIcon size={15} aria-hidden="true" />
      </IconButton>
    </section>
  );
};
