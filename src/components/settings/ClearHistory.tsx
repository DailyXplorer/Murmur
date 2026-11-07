import React, { useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { commands } from "@/bindings";
import { Button } from "../ui/Button";
import { Dialog } from "../ui/Dialog";
import { SettingContainer } from "../ui/SettingContainer";

interface ClearHistoryProps {
  grouped?: boolean;
}

/** Deletes every dictation that is not starred, and its recording, after a
 * confirmation. Starred dictations and meetings (their own library) are left
 * alone, which the dialog says. */
export const ClearHistory: React.FC<ClearHistoryProps> = ({
  grouped = false,
}) => {
  const { t } = useTranslation();
  const [open, setOpen] = useState(false);
  const [clearing, setClearing] = useState(false);

  const clear = async () => {
    setClearing(true);
    try {
      const result = await commands.clearHistory();
      if (result.status !== "ok") {
        throw new Error(result.error);
      }
      setOpen(false);
    } catch (error) {
      console.error("Failed to clear the history:", error);
      toast.error(t("settings.history.clear.error"));
    } finally {
      setClearing(false);
    }
  };

  return (
    <>
      <SettingContainer
        title={t("settings.history.clear.title")}
        description={t("settings.history.clear.description")}
        grouped={grouped}
      >
        <Button onClick={() => setOpen(true)}>
          {t("settings.history.clear.button")}
        </Button>
      </SettingContainer>
      <Dialog
        open={open}
        onOpenChange={(next) => {
          if (!clearing) setOpen(next);
        }}
        title={t("settings.history.clear.dialog.title")}
        closeLabel={t("common.close")}
        footer={
          <>
            <Button disabled={clearing} onClick={() => setOpen(false)}>
              {t("meetings.cancel")}
            </Button>
            <Button
              variant="destructive"
              disabled={clearing}
              onClick={() => void clear()}
            >
              {t("settings.history.clear.confirm")}
            </Button>
          </>
        }
      >
        <p className="text-sm text-text/80">
          {t("settings.history.clear.dialog.description")}
        </p>
      </Dialog>
    </>
  );
};
