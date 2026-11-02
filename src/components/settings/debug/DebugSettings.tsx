import React from "react";
import { useTranslation } from "react-i18next";
import { WordCorrectionThreshold } from "./WordCorrectionThreshold";
import { LogLevelSelector } from "./LogLevelSelector";
import { LiveLogViewer } from "./LiveLogViewer";
import { RecordingBuffer } from "./RecordingBuffer";
import { SettingsGroup } from "../../ui/SettingsGroup";
import { SettingsPage } from "../../ui/SettingsPage";
import { AlwaysOnMicrophone } from "../AlwaysOnMicrophone";
import { SoundPicker } from "../SoundPicker";
import { ClamshellMicrophoneSelector } from "../ClamshellMicrophoneSelector";
import { WhatsNewPreview } from "./WhatsNewPreview";
import { isWindows } from "@/lib/platform";

export const DebugSettings: React.FC = () => {
  const { t } = useTranslation();

  return (
    <SettingsPage label={t("sidebar.debug")}>
      <SettingsGroup title={t("settings.debug.title")}>
        <LogLevelSelector grouped={true} />
        {!isWindows && <WhatsNewPreview grouped={true} />}
        <SoundPicker
          label={t("settings.debug.soundTheme.label")}
          description={t("settings.debug.soundTheme.description")}
        />
        <WordCorrectionThreshold grouped={true} />
        <RecordingBuffer grouped={true} />
        <AlwaysOnMicrophone grouped={true} />
        {!isWindows && <ClamshellMicrophoneSelector grouped={true} />}
        <LiveLogViewer grouped={true} />
      </SettingsGroup>
    </SettingsPage>
  );
};
