import React, { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  commands,
  type MeetingAudioRetention,
  type MeetingsStorage,
} from "@/bindings";
import { useSettings } from "@/hooks/useSettings";
import { formatSize } from "@/lib/meetings/format";
import { Dropdown } from "../ui/Dropdown";
import { SettingContainer } from "../ui/SettingContainer";

const RETENTIONS: MeetingAudioRetention[] = [
  "keep",
  "days7",
  "days30",
  "days90",
];

/** How long processed meetings keep their audio, and the space meetings use
 * now. Their transcripts and summaries stay. */
export const MeetingsAudioRetention: React.FC = () => {
  const { t, i18n } = useTranslation();
  const { getSetting, updateSetting, isUpdating } = useSettings();
  const [storage, setStorage] = useState<MeetingsStorage | null>(null);

  useEffect(() => {
    let current = true;
    void commands.getMeetingsStorage().then((result) => {
      if (current && result.status === "ok") setStorage(result.data);
    });
    return () => {
      current = false;
    };
  }, []);

  const size = (bytes: number) => formatSize(bytes, i18n.language);
  const used =
    storage &&
    (storage.set_aside_bytes > 0
      ? t("settings.advanced.meetingsAudio.usedWithSetAside", {
          size: size(storage.audio_bytes),
          setAside: size(storage.set_aside_bytes),
        })
      : t("settings.advanced.meetingsAudio.used", {
          size: size(storage.audio_bytes),
        }));

  return (
    <SettingContainer
      title={t("settings.advanced.meetingsAudio.title")}
      description={t("settings.advanced.meetingsAudio.description")}
      grouped={true}
      detail={
        <div className="text-pretty text-xs text-text/60">
          <p>{t("settings.advanced.meetingsAudio.textKept")}</p>
          {used && <p>{used}</p>}
        </div>
      }
    >
      <Dropdown
        options={RETENTIONS.map((value) => ({
          value,
          label: t(`settings.advanced.meetingsAudio.options.${value}`),
        }))}
        selectedValue={getSetting("meeting_audio_retention") ?? "keep"}
        onSelect={(value) =>
          void updateSetting(
            "meeting_audio_retention",
            value as MeetingAudioRetention,
          )
        }
        disabled={isUpdating("meeting_audio_retention")}
      />
    </SettingContainer>
  );
};
