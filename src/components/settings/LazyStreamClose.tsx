import React from "react";
import { useTranslation } from "react-i18next";
import { ToggleSwitch } from "../ui/ToggleSwitch";
import { useSettings } from "../../hooks/useSettings";

interface LazyStreamCloseProps {
  grouped?: boolean;
}

export const LazyStreamClose: React.FC<LazyStreamCloseProps> = React.memo(
  ({ grouped = false }) => {
    const { t } = useTranslation();
    const { getSetting, updateSetting, isUpdating } = useSettings();

    const enabled = getSetting("lazy_stream_close") ?? false;

    return (
      <ToggleSwitch
        checked={enabled}
        onChange={(enabled) => updateSetting("lazy_stream_close", enabled)}
        isUpdating={isUpdating("lazy_stream_close")}
        label={t("settings.advanced.lazyStreamClose.label")}
        description={t("settings.advanced.lazyStreamClose.description")}
        grouped={grouped}
      />
    );
  },
);
