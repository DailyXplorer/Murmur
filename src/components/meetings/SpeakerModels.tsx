import React, { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import {
  commands,
  type AnalysisSupport,
  type LocalModelStatus,
} from "@/bindings";
import { downloadPercent } from "@/hooks/useLocalModel";
import { useSettings } from "@/hooks/useSettings";
import { formatSize } from "@/lib/meetings/format";
import { DeleteModelDialog } from "../settings/transcription/DeleteModelDialog";
import { useCodexStatus } from "../transcription/providerStatus";
import { Button } from "../ui/Button";
import { SettingContainer } from "../ui/SettingContainer";
import { SettingsGroup } from "../ui/SettingsGroup";

type Action = () => Promise<{ status: "ok" } | { status: "error" }>;

/** Download, cancel or delete the on-device speaker models, with progress.
 * They are the local models dictation uses too, so deleting them asks first
 * while dictation runs on this Mac. Their licences are in About. */
export const SpeakerModels: React.FC<{
  status: LocalModelStatus | null;
  /** Whether this Mac tells speakers apart. Intel Macs download the model
   * for dictation only. */
  analysis: AnalysisSupport | null;
}> = ({ status, analysis }) => {
  const { t, i18n } = useTranslation();
  const { getSetting } = useSettings();
  const { refreshStatus, status: account } = useCodexStatus();
  const [confirmDelete, setConfirmDelete] = useState(false);
  const dictationUsesIt = getSetting("transcription_engine") === "local";

  useEffect(() => {
    if (dictationUsesIt) void refreshStatus();
  }, [dictationUsesIt, refreshStatus]);

  if (!status || !analysis) return null;
  const { state } = status;
  const support = state.state === "unsupported" ? state.support : analysis;

  // The state, and with it the button's action, changes within milliseconds
  // of a click, so the rest of a double click would undo the first one.
  const run = async (event: React.MouseEvent, action: Action) => {
    if (event.detail > 1) return;
    try {
      const result = await action();
      if (result.status === "error") toast.error(t("meetings.models.error"));
    } catch (error) {
      console.error("Speaker models action failed:", error);
      toast.error(t("meetings.models.error"));
    }
  };

  if (state.state === "unsupported" || support !== "available") {
    return (
      <SettingsGroup title={t("meetings.models.title")}>
        <p className="px-4 py-3 text-sm text-text/60">
          {t(`meetings.models.${support}`)}
        </p>
      </SettingsGroup>
    );
  }

  const percent = downloadPercent(status);
  const size = formatSize(status.size_bytes, i18n.language);
  const label = {
    missing: t("meetings.models.missing"),
    downloading: t("meetings.models.downloading", { percent }),
    preparing: t("meetings.models.preparing"),
    ready: t("meetings.models.ready", { size }),
    failed: t("meetings.models.failed"),
  }[state.state];

  const deleteButton = (
    <Button
      onClick={(event) => {
        if (dictationUsesIt) setConfirmDelete(true);
        else void run(event, commands.deleteLocalModel);
      }}
    >
      {t("meetings.models.delete")}
    </Button>
  );
  // What a download that stopped left behind.
  const deleteIfKept = status.kept_bytes > 0 && deleteButton;

  const action = (() => {
    switch (state.state) {
      case "missing":
        return (
          <>
            <Button
              onClick={(event) => void run(event, commands.downloadLocalModel)}
            >
              {t("meetings.models.download")}
            </Button>
            {deleteIfKept}
          </>
        );
      case "failed":
        return (
          <>
            <Button
              onClick={(event) => void run(event, commands.downloadLocalModel)}
            >
              {t("meetings.models.retry")}
            </Button>
            {deleteIfKept}
          </>
        );
      case "downloading":
      case "preparing":
        return (
          <Button
            onClick={(event) =>
              void run(event, commands.cancelLocalModelDownload)
            }
          >
            {t("meetings.cancel")}
          </Button>
        );
      case "ready":
        return deleteButton;
    }
  })();

  return (
    <SettingsGroup title={t("meetings.models.title")}>
      <SettingContainer
        title={label}
        description={t("meetings.models.description", { size })}
        grouped={true}
        detail={
          state.state === "downloading" && (
            <div
              role="progressbar"
              aria-label={label}
              aria-valuemin={0}
              aria-valuemax={100}
              aria-valuenow={percent}
              className="mt-1 mb-1.5 h-1.5 overflow-hidden rounded-full bg-mid-gray/20"
            >
              <div
                className="h-full rounded-full bg-logo-primary transition-[width] duration-300"
                style={{ width: `${percent}%` }}
              />
            </div>
          )
        }
      >
        <div className="flex justify-end gap-2">{action}</div>
      </SettingContainer>
      <DeleteModelDialog
        open={confirmDelete}
        onOpenChange={setConfirmDelete}
        onConfirm={(event) => void run(event, commands.deleteLocalModel)}
        signedIn={account.kind !== "unavailable"}
        meetingsUseIt={true}
      />
    </SettingsGroup>
  );
};
