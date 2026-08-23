import React, { useEffect } from "react";
import { Check } from "lucide-react";
import { useTranslation } from "react-i18next";
import { SettingContainer } from "../ui/SettingContainer";
import { useSettings } from "@/hooks/useSettings";
import {
  ACCENT_COLOR_OPTIONS,
  ACCENT_COLOR_PREVIEWS,
  applyAccentColor,
  getStoredAccentColor,
} from "@/lib/utils/accentColor";
import type { AccentColor } from "@/bindings";

interface AccentColorSelectorProps {
  descriptionMode?: "inline" | "tooltip";
  grouped?: boolean;
}

export const AccentColorSelector: React.FC<AccentColorSelectorProps> =
  React.memo(({ descriptionMode = "tooltip", grouped = false }) => {
    const { t } = useTranslation();
    const { settings, updateSetting, isUpdating } = useSettings();
    const settingsLoaded = settings !== null;
    const currentAccent: AccentColor = settings
      ? (settings.accent_color ?? "pink")
      : getStoredAccentColor();

    useEffect(() => {
      if (!settingsLoaded) return;
      applyAccentColor(currentAccent);
    }, [currentAccent, settingsLoaded]);

    const handleAccentChange = (accentColor: AccentColor) => {
      applyAccentColor(accentColor);
      void updateSetting("accent_color", accentColor);
    };

    return (
      <SettingContainer
        title={t("accentColor.title")}
        description={t("accentColor.description")}
        descriptionMode={descriptionMode}
        grouped={grouped}
      >
        <div
          className="flex items-center gap-1"
          role="radiogroup"
          aria-label={t("accentColor.title")}
        >
          {ACCENT_COLOR_OPTIONS.map((accentColor) => {
            const selected = accentColor === currentAccent;
            const label = t(`accentColor.options.${accentColor}`);
            const disabled = !settingsLoaded || isUpdating("accent_color");

            return (
              <label
                key={accentColor}
                title={label}
                className={`relative flex size-10 shrink-0 items-center justify-center rounded-full transition-[transform,box-shadow] duration-150 ${
                  disabled
                    ? "cursor-wait opacity-60"
                    : "cursor-pointer active:scale-[0.96]"
                } ${
                  selected
                    ? "shadow-[0_0_0_2px_var(--color-background),0_0_0_4px_var(--color-text)]"
                    : "hover:shadow-[0_0_0_3px_color-mix(in_srgb,var(--color-text)_14%,transparent)]"
                }`}
              >
                <input
                  type="radio"
                  name="accent-color"
                  value={accentColor}
                  checked={selected}
                  aria-label={label}
                  disabled={disabled}
                  onChange={() => handleAccentChange(accentColor)}
                  className="peer absolute inset-0 m-0 size-full cursor-inherit opacity-0"
                />
                <span
                  className="pointer-events-none flex size-7 items-center justify-center rounded-full shadow-[inset_0_0_0_1px_rgba(0,0,0,0.1)] peer-focus-visible:outline-none peer-focus-visible:ring-2 peer-focus-visible:ring-text peer-focus-visible:ring-offset-2 peer-focus-visible:ring-offset-background dark:shadow-[inset_0_0_0_1px_rgba(255,255,255,0.1)]"
                  style={{
                    backgroundColor: ACCENT_COLOR_PREVIEWS[accentColor],
                  }}
                >
                  {selected && (
                    <Check
                      className="size-4 text-[#0f0f0f]"
                      strokeWidth={3}
                      aria-hidden="true"
                    />
                  )}
                </span>
              </label>
            );
          })}
        </div>
      </SettingContainer>
    );
  });

AccentColorSelector.displayName = "AccentColorSelector";
