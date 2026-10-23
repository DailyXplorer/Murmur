import React, { useState } from "react";
import { useTranslation } from "react-i18next";
import { useSettings } from "../../hooks/useSettings";
import { Input } from "../ui/Input";
import { SettingContainer } from "../ui/SettingContainer";

const MIN_HISTORY_LIMIT = 1;
const MAX_HISTORY_LIMIT = 1000;

interface HistoryLimitProps {
  descriptionMode?: "tooltip" | "inline";
  grouped?: boolean;
}

export const HistoryLimit: React.FC<HistoryLimitProps> = ({
  descriptionMode = "inline",
  grouped = false,
}) => {
  const { t } = useTranslation();
  const { getSetting, updateSetting } = useSettings();

  const historyLimit = getSetting("history_limit") ?? 5;
  const [draft, setDraft] = useState<string | null>(null);

  // Lowering the limit deletes history at once, so a half-typed number
  // must never be saved: the value is committed on Enter or blur only.
  const commit = (input: HTMLInputElement) => {
    if (draft === null) return;
    if (draft === "") {
      setDraft(null);
      return;
    }
    // Use the number control's complete value: parseInt("1e3") is 1,
    // not 1000. Never round or clamp a value that deletes history.
    const value = input.valueAsNumber;
    if (
      !input.validity.valid ||
      !Number.isInteger(value) ||
      value < MIN_HISTORY_LIMIT ||
      value > MAX_HISTORY_LIMIT
    ) {
      input.reportValidity();
      return;
    }
    setDraft(null);
    if (value !== historyLimit) {
      void updateSetting("history_limit", value);
    }
  };

  const handleKeyDown = (event: React.KeyboardEvent<HTMLInputElement>) => {
    if (event.key === "Enter") {
      commit(event.currentTarget);
    } else if (event.key === "Escape") {
      setDraft(null);
    }
  };

  return (
    <SettingContainer
      title={t("settings.debug.historyLimit.title")}
      description={t("settings.debug.historyLimit.description")}
      descriptionMode={descriptionMode}
      grouped={grouped}
      layout="horizontal"
    >
      <div className="flex items-center gap-2">
        <Input
          type="number"
          min={MIN_HISTORY_LIMIT}
          max={MAX_HISTORY_LIMIT}
          step={1}
          value={draft ?? String(historyLimit)}
          onChange={(event) => setDraft(event.target.value)}
          onBlur={(event) => commit(event.currentTarget)}
          onKeyDown={handleKeyDown}
          className="min-w-0 flex-1"
        />
        <span className="text-sm text-text">
          {t("settings.debug.historyLimit.entries")}
        </span>
      </div>
    </SettingContainer>
  );
};
