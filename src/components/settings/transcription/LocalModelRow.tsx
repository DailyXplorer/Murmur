import React, { useId, useState } from "react";
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
import { DeleteModelDialog } from "./DeleteModelDialog";

type Action = () => Promise<{ status: "ok" } | { status: "error" }>;

/** Why the download stopped, in the user's words. */
export const failureText = (
  t: TFunction,
  reason: DownloadFailure,
  locale: string,
  keys = "settings.transcription.localModel.failed",
): string =>
  reason.kind === "not_enough_space"
    ? t(`${keys}.not_enough_space`, {
        needed: formatSize(reason.needed_bytes, locale),
        free: formatSize(reason.free_bytes, locale),
      })
    : t(`${keys}.${reason.kind}`);

const ROW = "settings.transcription.localModel.row";

const FAILURE_TITLES: Record<DownloadFailure["kind"], string> = {
  no_internet: `${ROW}.stopped`,
  server_unavailable: `${ROW}.stopped`,
  damaged: `${ROW}.stopped`,
  not_enough_space: `${ROW}.noSpace`,
  disk: `${ROW}.stopped`,
  prepare_failed: `${ROW}.loadFailed`,
};

/** The row's two lines: the model's state, then at most one short note. */
const rowLines = (
  t: TFunction,
  status: LocalModelStatus,
  meetingsUseIt: boolean,
  locale: string,
): [string, string | null] => {
  const meetings = meetingsUseIt ? t(`${ROW}.meetings`) : null;
  const { state } = status;
  switch (state.state) {
    case "unsupported":
      return [t(`${ROW}.unavailable`), t(`${ROW}.needs.${state.support}`)];
    case "missing":
      return [t("settings.transcription.localModel.missing"), meetings];
    case "downloading":
      return [
        t(`${ROW}.downloading`, { percent: downloadPercent(status) }),
        meetings,
      ];
    case "preparing":
      return [t(`${ROW}.preparing`), t(`${ROW}.preparingNote`)];
    case "ready":
      return [
        t("settings.transcription.localModel.ready", {
          size: formatSize(status.size_bytes, locale),
        }),
        meetings,
      ];
    case "failed":
      return [
        t(FAILURE_TITLES[state.reason.kind]),
        failureText(t, state.reason, locale, `${ROW}.reason`),
      ];
    default: {
      const _exhaustive: never = state;
      return _exhaustive;
    }
  }
};

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

/** The local model's state with its actions: download, cancel, try again or
 * delete, which also frees what a download that stopped left behind.
 * Deleting asks first, saying what stops working. */
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
  const statusId = useId();
  const { state } = status;
  const size = formatSize(status.size_bytes, i18n.language);
  const [label, note] = rowLines(t, status, meetingsUseIt, i18n.language);

  const deleteButton = (
    <Button onClick={() => setConfirmDelete(true)}>
      {t("settings.transcription.localModel.delete")}
    </Button>
  );
  const deleteIfKept = status.kept_bytes > 0 && deleteButton;

  const action = (() => {
    switch (state.state) {
      case "missing":
        return (
          <>
            <Button
              onClick={(event) => void run(event, commands.downloadLocalModel)}
            >
              {t("settings.transcription.localModel.download", { size })}
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
              {t("settings.transcription.localModel.retry")}
            </Button>
            {deleteIfKept}
          </>
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
        return deleteButton;
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
      <div className="flex min-h-12 items-center justify-between gap-5 px-4 py-1">
        <div className="min-w-0 flex-1">
          <p id={statusId} className="text-pretty text-sm font-medium">
            {label}
          </p>
          {state.state === "downloading" && (
            <div
              role="progressbar"
              aria-labelledby={statusId}
              aria-valuemin={0}
              aria-valuemax={100}
              aria-valuenow={downloadPercent(status)}
              className="mt-1 mb-1.5 h-1.5 overflow-hidden rounded-full bg-mid-gray/20"
            >
              <div
                className="h-full rounded-full bg-logo-primary transition-[width] duration-300"
                style={{ width: `${downloadPercent(status)}%` }}
              />
            </div>
          )}
          {note && <p className="text-pretty text-xs text-text/50">{note}</p>}
        </div>
        <div className="relative w-[var(--settings-control-rail-width)] shrink-0 text-end">
          <div className="flex justify-end gap-2">{action}</div>
        </div>
      </div>
      <DeleteModelDialog
        open={confirmDelete}
        onOpenChange={setConfirmDelete}
        onConfirm={(event) => void run(event, commands.deleteLocalModel)}
        signedIn={signedIn}
        meetingsUseIt={meetingsUseIt}
      />
    </div>
  );
};
