import React, { useEffect } from "react";
import { CheckIcon } from "@phosphor-icons/react/dist/csr/Check";
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
    const accentColorUpdating = isUpdating("accent_color");
    const currentAccent: AccentColor = settings
      ? (settings.accent_color ?? "pink")
      : getStoredAccentColor();

    useEffect(() => {
      if (!settingsLoaded || accentColorUpdating) return;
      applyAccentColor(currentAccent);
    }, [accentColorUpdating, currentAccent, settingsLoaded]);

    const handleAccentChange = (accentColor: AccentColor) => {
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
            const disabled = !settingsLoaded || accentColorUpdating;

            return (
              <label
                key={accentColor}
                title={label}
                className={`group relative flex size-control shrink-0 items-center justify-center rounded-full transition-transform duration-150 ${
                  disabled
                    ? "cursor-wait opacity-60"
                    : "cursor-pointer active:scale-[0.96]"
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
                {/* Selected, the swatch wears the range sliders' white knob
                    edge: a white gap, a thin neutral line, a soft shadow. */}
                <span
                  className={`pointer-events-none flex size-6 items-center justify-center rounded-full ring-1 ring-black/10 ring-inset transition-shadow duration-150 peer-focus-visible:outline-2 peer-focus-visible:outline-offset-4 peer-focus-visible:outline-logo-primary dark:ring-white/10 ${
                    selected
                      ? "shadow-[0_0_0_2px_#fff,0_0_0_3px_rgb(0_0_0/0.18),0_1px_3px_3px_rgb(0_0_0/0.2)]"
                      : disabled
                        ? ""
                        : "group-hover:shadow-[0_0_0_3px_color-mix(in_srgb,var(--color-text)_14%,transparent)]"
                  }`}
                  style={{
                    backgroundColor: ACCENT_COLOR_PREVIEWS[accentColor],
                  }}
                >
                  {selected && (
                    <CheckIcon
                      className="size-3.5 text-[#0f0f0f]"
                      weight="bold"
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
