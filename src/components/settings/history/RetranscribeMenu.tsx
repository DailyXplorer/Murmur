import React, { useCallback, useEffect, useRef, useState } from "react";
import { CaretDownIcon } from "@phosphor-icons/react/dist/csr/CaretDown";
import { useTranslation } from "react-i18next";
import { commands } from "@/bindings";
import { useLanguageName } from "../../../hooks/useLanguageName";
import {
  SELECTABLE_LANGUAGES,
  supportsLanguageCode,
} from "../../../lib/constants/languages";
import { FloatingPanel } from "../../ui/FloatingPanel";
import { IconButton } from "../../ui/IconButton";
import { DescriptionTooltip } from "../../ui/SettingContainer";

interface RetranscribeMenuProps {
  disabled: boolean;
  /** Runs the transcription in `language` ("auto" or a code), this once. */
  onPick: (language: string) => void;
}

/**
 * A caret beside "Transcribe again" that lists the languages the chosen engine
 * transcribes. The pick is for that one run; the Language setting is never
 * touched. The list is read when the menu opens, since the engine can change
 * between two openings.
 */
export const RetranscribeMenu: React.FC<RetranscribeMenuProps> = ({
  disabled,
  onPick,
}) => {
  const { t } = useTranslation();
  const languageName = useLanguageName();
  const [open, setOpen] = useState(false);
  const [supported, setSupported] = useState<string[] | null>(null);
  const anchorRef = useRef<HTMLSpanElement>(null);

  useEffect(() => {
    if (!open) return;
    let current = true;
    commands
      .getRetranscribeLanguages()
      .then((languages) => {
        if (current) setSupported(languages);
      })
      .catch((error) => {
        console.error("Failed to read the languages:", error);
        if (current) setSupported(null);
      });
    return () => {
      current = false;
    };
  }, [open]);

  useEffect(() => {
    if (disabled) setOpen(false);
  }, [disabled]);

  const languages = SELECTABLE_LANGUAGES.filter(
    (language) =>
      language.value === "auto" ||
      supported === null ||
      supportsLanguageCode(supported, language.value),
  ).map((language) => ({
    value: language.value,
    name: languageName(language.value),
  }));
  const dismiss = useCallback(() => setOpen(false), []);
  const pick = (language: string) => {
    setOpen(false);
    onPick(language);
  };

  return (
    <>
      <span ref={anchorRef} className="inline-flex">
        <IconButton
          onClick={() => setOpen((isOpen) => !isOpen)}
          disabled={disabled}
          active={open}
          label={t("settings.history.retranscribeIn")}
          aria-expanded={open}
        >
          <CaretDownIcon size={11} aria-hidden="true" />
        </IconButton>
      </span>
      <FloatingPanel
        open={open && !disabled}
        anchorRef={anchorRef}
        onDismiss={dismiss}
        focusFirstOptionOnOpen={true}
        className="flex min-w-[220px] flex-col overflow-hidden rounded-md border border-mid-gray/80 bg-background shadow-lg"
      >
        <div className="flex items-center gap-0.5 border-b border-mid-gray/40 ps-3 pe-1">
          <h4 className="min-w-0 flex-1 truncate text-sm font-medium">
            {t("settings.history.retranscribeMenuTitle")}
          </h4>
          <DescriptionTooltip
            description={t("settings.history.retranscribeMenuInfo")}
            position="bottom"
          />
        </div>
        <div
          role="listbox"
          aria-label={t("settings.history.retranscribeMenuTitle")}
          className="min-h-0 flex-1 overflow-y-auto"
        >
          {languages.map((language) => (
            <button
              key={language.value}
              type="button"
              role="option"
              aria-selected={false}
              tabIndex={-1}
              className="min-h-control w-full px-3 py-1 text-start text-sm font-normal transition-colors duration-150 hover:bg-logo-primary/10 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-accent-text"
              onClick={() => pick(language.value)}
            >
              <span className="truncate">{language.name}</span>
            </button>
          ))}
        </div>
      </FloatingPanel>
    </>
  );
};
