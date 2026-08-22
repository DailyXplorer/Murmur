import React from "react";
import { useTranslation } from "react-i18next";
import { SettingsGroup } from "../../ui/SettingsGroup";
import { LanguageSelector } from "../LanguageSelector";
import { useModelStore } from "../../../stores/modelStore";
import type { ModelInfo } from "@/bindings";

export const ModelSettingsCard: React.FC = () => {
  const { t } = useTranslation();
  const { currentModel, models } = useModelStore();
  const currentModelInfo = models.find((m: ModelInfo) => m.id === currentModel);

  if (!currentModel || !currentModelInfo?.supports_language_selection) {
    return null;
  }

  return (
    <SettingsGroup
      title={t("settings.modelSettings.title", {
        model: currentModelInfo.name,
      })}
    >
      <LanguageSelector
        descriptionMode="tooltip"
        grouped={true}
        supportedLanguages={currentModelInfo.supported_languages}
        supportsLanguageDetection={currentModelInfo.supports_language_detection}
      />
    </SettingsGroup>
  );
};
