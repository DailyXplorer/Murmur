import React, { useEffect, useState } from "react";
import { CopyIcon } from "@phosphor-icons/react/dist/csr/Copy";
import { useTranslation } from "react-i18next";
import { commands, type HistoryVersion } from "@/bindings";
import { formatDateTime } from "@/utils/dateFormat";
import { IconButton, IconToolbar } from "../../ui/IconButton";
import { DescriptionTooltip } from "../../ui/SettingContainer";
import { EngineLabel } from "./EngineLabel";

interface HistoryVersionsProps {
  entryId: number;
  /** How many earlier transcripts the entry has. */
  count: number;
  /**
   * The entry's current text: a restore swaps texts without changing `count`,
   * so a change here reloads the list as well.
   */
  currentText: string;
  disabled: boolean;
  copyText: (text: string) => Promise<void>;
  restore: (versionId: number) => Promise<void>;
}

/** The earlier transcripts of a dictation, behind a quiet toggle. */
export const HistoryVersions: React.FC<HistoryVersionsProps> = ({
  entryId,
  count,
  currentText,
  disabled,
  copyText,
  restore,
}) => {
  const { t, i18n } = useTranslation();
  const [open, setOpen] = useState(false);
  const [versions, setVersions] = useState<HistoryVersion[] | null>(null);
  const [failed, setFailed] = useState(false);

  useEffect(() => {
    if (!open || count === 0) return;
    let current = true;
    setFailed(false);
    commands
      .getHistoryVersions(entryId)
      .then((result) => {
        if (!current) return;
        if (result.status === "ok") setVersions(result.data);
        else setFailed(true);
      })
      .catch(() => current && setFailed(true));
    return () => {
      current = false;
    };
  }, [open, entryId, count, currentText]);

  if (count === 0) return null;

  return (
    <div className="-mt-3 flex flex-col gap-2">
      {/* As tall as its text: the bubble's hit area overflows the line. */}
      <div className="flex h-4 items-center gap-0.5">
        <button
          type="button"
          className="text-xs text-text/50 underline-offset-2 hover:underline"
          onClick={() => setOpen((isOpen) => !isOpen)}
          aria-expanded={open}
        >
          {open
            ? t("settings.history.versionsHide")
            : t("settings.history.versionsShow", { count })}
        </button>
        <DescriptionTooltip
          description={t("settings.history.versionsInfo")}
          position="top"
        />
      </div>
      {open && failed && (
        <p className="text-xs text-text/50">
          {t("settings.history.versionsLoadError")}
        </p>
      )}
      {open && !failed && versions && (
        <ul className="flex flex-col gap-2">
          {versions.map((version) => (
            <li
              key={version.id}
              className="flex flex-col gap-1 rounded-md border border-mid-gray/20 px-3 py-2"
            >
              <div className="flex items-center justify-between gap-2">
                <p className="min-w-0 truncate text-xs text-text/50">
                  {formatDateTime(String(version.produced_at), i18n.language)}
                  <span aria-hidden="true" className="mx-1.5">
                    ·
                  </span>
                  <EngineLabel engine={version.engine} />
                </p>
                <IconToolbar>
                  <IconButton
                    onClick={() => void copyText(version.transcription_text)}
                    label={t("settings.history.copyVersion")}
                  >
                    <CopyIcon size={14} aria-hidden="true" />
                  </IconButton>
                  <button
                    type="button"
                    disabled={disabled}
                    className="h-7 cursor-pointer rounded-md px-2 text-xs font-medium text-accent-text transition-colors hover:bg-logo-primary/15 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent-text disabled:cursor-not-allowed disabled:text-text/25 disabled:hover:bg-transparent"
                    aria-label={t("settings.history.restoreVersion")}
                    onClick={() => void restore(version.id)}
                  >
                    {t("settings.history.restore")}
                  </button>
                </IconToolbar>
              </div>
              <p className="select-text cursor-text whitespace-pre-wrap break-words text-sm italic text-text/70">
                {version.transcription_text}
              </p>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
};
