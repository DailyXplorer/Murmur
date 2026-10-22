import React from "react";
import { useTranslation } from "react-i18next";
import { Dialog } from "../ui/Dialog";

const LICENCES = ["parakeet", "diarization", "fluidaudio"] as const;

/** The on-device models' licences and attributions. */
export const ModelLicencesDialog: React.FC<{
  open: boolean;
  onOpenChange: (open: boolean) => void;
}> = ({ open, onOpenChange }) => {
  const { t } = useTranslation();
  return (
    <Dialog
      open={open}
      onOpenChange={onOpenChange}
      title={t("meetings.models.licences")}
      closeLabel={t("common.close")}
    >
      <ul className="list-disc space-y-2 pl-5 text-sm text-text/80">
        {LICENCES.map((key) => (
          <li key={key}>{t(`meetings.models.licence.${key}`)}</li>
        ))}
      </ul>
      <p className="mt-3 text-sm text-text/60">
        {t("meetings.models.licencesNote")}
      </p>
    </Dialog>
  );
};
