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
  const commit = () => {
    if (draft === null) return;
    setDraft(null);
    const value = parseInt(draft, 10);
    if (isNaN(value)) return;
    const limit = Math.min(
      MAX_HISTORY_LIMIT,
      Math.max(MIN_HISTORY_LIMIT, value),
    );
    if (limit !== historyLimit) {
      void updateSetting("history_limit", limit);
    }
  };

  const handleKeyDown = (event: React.KeyboardEvent<HTMLInputElement>) => {
    if (event.key === "Enter") {
      commit();
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
      <div className="flex items-center justify-end space-x-2">
        <Input
          type="number"
          min={MIN_HISTORY_LIMIT}
          max={MAX_HISTORY_LIMIT}
          value={draft ?? historyLimit}
          onChange={(event) => setDraft(event.target.value)}
          onBlur={commit}
          onKeyDown={handleKeyDown}
          className="w-20"
        />
        <span className="text-sm text-text">
          {t("settings.debug.historyLimit.entries")}
        </span>
      </div>
    </SettingContainer>
  );
};
