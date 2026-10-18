import React from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { commands, type MeetingModelsStatus } from "@/bindings";
import { formatSize } from "@/lib/meetings/format";
import { Button } from "../ui/Button";
import { SettingContainer } from "../ui/SettingContainer";
import { SettingsGroup } from "../ui/SettingsGroup";

type Action = () => Promise<{ status: "ok" } | { status: "error" }>;

/** Download, cancel or delete the on-device speaker models, with progress.
 * Their licences are in About. */
export const SpeakerModels: React.FC<{
  status: MeetingModelsStatus | null;
}> = ({ status }) => {
  const { t, i18n } = useTranslation();
  if (!status) return null;
  const { state } = status;

  const run = async (action: Action) => {
    try {
      const result = await action();
      if (result.status === "error") toast.error(t("meetings.models.error"));
    } catch (error) {
      console.error("Speaker models action failed:", error);
      toast.error(t("meetings.models.error"));
    }
  };

  if (state.state === "unsupported") {
    return (
      <SettingsGroup title={t("meetings.models.title")}>
        <p className="px-4 py-3 text-sm text-text/60">
          {t(`meetings.models.${state.support}`)}
        </p>
      </SettingsGroup>
    );
  }

  const percent =
    state.state === "downloading" ? Math.round(state.fraction * 100) : 0;
  const label = {
    missing: t("meetings.models.missing"),
    downloading: t("meetings.models.downloading", { percent }),
    preparing: t("meetings.models.preparing"),
    ready: t("meetings.models.ready", {
      size: formatSize(status.bytes_on_disk, i18n.language),
    }),
    failed: t("meetings.models.failed"),
  }[state.state];

  const action = (() => {
    switch (state.state) {
      case "missing":
        return (
          <Button onClick={() => void run(commands.downloadMeetingModels)}>
            {t("meetings.models.download")}
          </Button>
        );
      case "failed":
        return (
          <Button onClick={() => void run(commands.downloadMeetingModels)}>
            {t("meetings.models.retry")}
          </Button>
        );
      case "downloading":
      case "preparing":
        return (
          <Button
            onClick={() => void run(commands.cancelMeetingModelsDownload)}
          >
            {t("meetings.cancel")}
          </Button>
        );
      case "ready":
        return (
          <Button onClick={() => void run(commands.deleteMeetingModels)}>
            {t("meetings.models.delete")}
          </Button>
        );
    }
  })();

  return (
    <SettingsGroup title={t("meetings.models.title")}>
      <SettingContainer
        title={label}
        description={t("meetings.models.description")}
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
    </SettingsGroup>
  );
};
