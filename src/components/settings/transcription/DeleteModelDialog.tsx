import React from "react";
import { useTranslation } from "react-i18next";
import { Button } from "../../ui/Button";
import { Dialog } from "../../ui/Dialog";

/** Asks before the local model is deleted, saying what stops working.
 * Transcription and Meetings delete the same files. */
export const DeleteModelDialog: React.FC<{
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onConfirm: (event: React.MouseEvent) => void;
  /** Without an account nothing takes over once the model is gone. */
  signedIn: boolean;
  /** Meetings use the same model to tell speakers apart on this Mac. */
  meetingsUseIt: boolean;
}> = ({ open, onOpenChange, onConfirm, signedIn, meetingsUseIt }) => {
  const { t } = useTranslation();
  return (
    <Dialog
      open={open}
      onOpenChange={onOpenChange}
      title={t("settings.transcription.localModel.deleteDialog.title")}
      closeLabel={t("common.close")}
      footer={
        <>
          <Button onClick={() => onOpenChange(false)}>
            {t("meetings.cancel")}
          </Button>
          <Button
            onClick={(event) => {
              onOpenChange(false);
              onConfirm(event);
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
          <p>{t("settings.transcription.localModel.deleteDialog.meetings")}</p>
        )}
      </div>
    </Dialog>
  );
};
