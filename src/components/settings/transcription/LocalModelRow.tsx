import React, { useState } from "react";
import { useTranslation } from "react-i18next";
import type { TFunction } from "i18next";
import { toast } from "sonner";
import {
  commands,
  type DownloadFailure,
  type LocalModelStatus,
} from "@/bindings";
import { downloadPercent } from "@/hooks/useLocalModel";
import { formatSize } from "@/lib/meetings/format";
import { Button } from "../../ui/Button";
import { Dialog } from "../../ui/Dialog";
import { SettingContainer } from "../../ui/SettingContainer";

type Action = () => Promise<{ status: "ok" } | { status: "error" }>;

/** Why the download stopped, in the user's words. */
export const failureText = (
  t: TFunction,
  reason: DownloadFailure,
  locale: string,
): string =>
  reason.kind === "not_enough_space"
    ? t("settings.transcription.localModel.failed.not_enough_space", {
        needed: formatSize(reason.needed_bytes, locale),
        free: formatSize(reason.free_bytes, locale),
      })
    : t(`settings.transcription.localModel.failed.${reason.kind}`);

/** Runs a model command once per click, with a toast if it fails. The
 * state, and with it the button, changes within milliseconds of a click, so
 * the rest of a double click would undo the first one. */
export const useModelAction = (errorKey: string) => {
  const { t } = useTranslation();
  return async (event: React.MouseEvent, action: Action) => {
    if (event.detail > 1) return;
    try {
      const result = await action();
      if (result.status === "error") toast.error(t(errorKey));
    } catch (error) {
      console.error("Local model action failed:", error);
      toast.error(t(errorKey));
    }
  };
};

/** The local model's state with the one action it allows: download, cancel,
 * try again or delete. Deleting asks first, saying what stops working. */
export const LocalModelRow: React.FC<{
  status: LocalModelStatus;
  /** Without an account nothing takes over once the model is gone. */
  signedIn: boolean;
  /** Meetings use the same model to tell speakers apart on this Mac. */
  meetingsUseIt: boolean;
}> = ({ status, signedIn, meetingsUseIt }) => {
  const { t, i18n } = useTranslation();
  const run = useModelAction("settings.transcription.localModel.error");
  const [confirmDelete, setConfirmDelete] = useState(false);
  const { state } = status;
  const size = formatSize(status.size_bytes, i18n.language);
  const percent = downloadPercent(status);

  const label = (() => {
    switch (state.state) {
      case "unsupported":
        return t(`settings.transcription.engine.${state.support}`);
      case "missing":
        return t("settings.transcription.localModel.missing");
      case "downloading":
        return t("settings.transcription.localModel.downloading", {
          done: new Intl.NumberFormat(i18n.language, {
            maximumFractionDigits: 0,
          }).format(state.done_bytes / 1_000_000),
          total: formatSize(state.total_bytes, i18n.language),
        });
      case "preparing":
        return t("settings.transcription.localModel.preparing");
      case "ready":
        return t("settings.transcription.localModel.ready", { size });
      case "failed":
        return failureText(t, state.reason, i18n.language);
      default: {
        const _exhaustive: never = state;
        return _exhaustive;
      }
    }
  })();

  const action = (() => {
    switch (state.state) {
      case "missing":
        return (
          <Button
            onClick={(event) => void run(event, commands.downloadLocalModel)}
          >
            {t("settings.transcription.localModel.download", { size })}
          </Button>
        );
      case "failed":
        return (
          <Button
            onClick={(event) => void run(event, commands.downloadLocalModel)}
          >
            {t("settings.transcription.localModel.retry")}
          </Button>
        );
      case "downloading":
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
        return (
          <Button onClick={() => setConfirmDelete(true)}>
            {t("settings.transcription.localModel.delete")}
          </Button>
        );
      case "preparing":
      case "unsupported":
        return null;
      default: {
        const _exhaustive: never = state;
        return _exhaustive;
      }
    }
  })();

  return (
    <div data-testid="local-model-row">
      <SettingContainer
        title={t("settings.transcription.localModel.title")}
        description={t("settings.transcription.localModel.description")}
        grouped={true}
        detail={
          <>
            <p className="text-pretty text-sm text-text/70">{label}</p>
            {state.state === "downloading" && (
              <div
                role="progressbar"
                aria-label={t("settings.transcription.localModel.title")}
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
            )}
            {meetingsUseIt && (
              <p className="text-pretty text-xs text-text/50">
                {t("settings.transcription.localModel.meetings")}
              </p>
            )}
          </>
        }
      >
        <div className="flex justify-end gap-2">{action}</div>
      </SettingContainer>
      <Dialog
        open={confirmDelete}
        onOpenChange={setConfirmDelete}
        title={t("settings.transcription.localModel.deleteDialog.title")}
        closeLabel={t("common.close")}
        footer={
          <>
            <Button onClick={() => setConfirmDelete(false)}>
              {t("meetings.cancel")}
            </Button>
            <Button
              onClick={(event) => {
                setConfirmDelete(false);
                void run(event, commands.deleteLocalModel);
              }}
            >
              {t("settings.transcription.localModel.deleteDialog.confirm")}
            </Button>
          </>
        }
      >
        <div className="space-y-2 text-sm text-text/80">
          <p>
            {signedIn
              ? t("settings.transcription.localModel.deleteDialog.withAccount")
              : t(
                  "settings.transcription.localModel.deleteDialog.withoutAccount",
                )}
          </p>
          {meetingsUseIt && (
            <p>
              {t("settings.transcription.localModel.deleteDialog.meetings")}
            </p>
          )}
        </div>
      </Dialog>
    </div>
  );
};
