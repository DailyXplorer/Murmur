import React from "react";
import { useTranslation } from "react-i18next";
import { useSettings } from "@/hooks/useSettings";
import { isWindows } from "@/lib/platform";
import { Dialog } from "../ui/Dialog";

// Windows dictates with the speech model alone. The speaker model and its
// runtime are downloaded and linked for Meetings, so a user with Meetings off
// is not credited for what Murmur never fetches for them.
const MAC_LICENCES = ["parakeet", "diarization", "fluidaudio"] as const;
const WINDOWS_LICENCES = ["parakeet"] as const;
const WINDOWS_MEETINGS_LICENCES = [
  "parakeet",
  "pyannote",
  "onnxruntime",
] as const;

/** The on-device models' licences and attributions. */
export const ModelLicencesDialog: React.FC<{
  open: boolean;
  onOpenChange: (open: boolean) => void;
}> = ({ open, onOpenChange }) => {
  const { t } = useTranslation();
  const { getSetting } = useSettings();
  const licences = !isWindows
    ? MAC_LICENCES
    : getSetting("meetings_enabled") === true
      ? WINDOWS_MEETINGS_LICENCES
      : WINDOWS_LICENCES;
  return (
    <Dialog
      open={open}
      onOpenChange={onOpenChange}
      title={t("meetings.models.licences")}
      closeLabel={t("common.close")}
    >
      <ul className="list-disc space-y-2 pl-5 text-sm text-text/80">
        {licences.map((key) => (
          <li key={key}>{t(`meetings.models.licence.${key}`)}</li>
        ))}
      </ul>
      <p className="mt-3 text-sm text-text/60">
        {t("meetings.models.licencesNote")}
      </p>
    </Dialog>
  );
};
