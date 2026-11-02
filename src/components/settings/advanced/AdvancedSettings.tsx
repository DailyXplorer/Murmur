import React from "react";
import { useTranslation } from "react-i18next";
import { ShowOverlay } from "../ShowOverlay";
import { SettingsGroup } from "../../ui/SettingsGroup";
import { SettingsPage } from "../../ui/SettingsPage";
import { StartHidden } from "../StartHidden";
import { AutostartToggle } from "../AutostartToggle";
import { ShowTrayIcon } from "../ShowTrayIcon";
import { ExperimentalToggle } from "../ExperimentalToggle";
import { MeetingsToggle } from "../MeetingsToggle";
import { MeetingsAudioRetention } from "../MeetingsAudioRetention";
import { MeetingCallPrompt } from "../MeetingCallPrompt";
import { McpAccess } from "../McpAccess";
import { MeetingLiveTranscript } from "../MeetingLiveTranscript";
import { useSettings } from "../../../hooks/useSettings";
import { LazyStreamClose } from "../LazyStreamClose";
import { ShowWhatsNewOnUpdate } from "../ShowWhatsNewOnUpdate";
import { UpdateChecksToggle } from "../UpdateChecksToggle";
import { AppDataDirectory } from "../AppDataDirectory";
import { LogDirectory } from "../debug";

export const AdvancedSettings: React.FC = () => {
  const { t } = useTranslation();
  const { getSetting } = useSettings();
  const experimentalEnabled = getSetting("experimental_enabled") || false;
  const meetingsEnabled = getSetting("meetings_enabled") ?? true;

  return (
    <SettingsPage label={t("sidebar.advanced")}>
      <SettingsGroup title={t("settings.advanced.groups.app")}>
        <StartHidden grouped={true} />
        <AutostartToggle grouped={true} />
        <ShowTrayIcon grouped={true} />
        <ShowOverlay grouped={true} />
        <MeetingsToggle grouped={true} />
        {meetingsEnabled && <MeetingCallPrompt />}
        {meetingsEnabled && <MeetingLiveTranscript />}
        {meetingsEnabled && <MeetingsAudioRetention />}
      </SettingsGroup>

      <SettingsGroup title={t("settings.advanced.groups.mcp")}>
        <McpAccess />
      </SettingsGroup>

      <SettingsGroup title={t("settings.advanced.groups.updates")}>
        <UpdateChecksToggle grouped={true} />
        <ShowWhatsNewOnUpdate grouped={true} />
      </SettingsGroup>

      <SettingsGroup title={t("settings.advanced.groups.storage")}>
        <AppDataDirectory grouped={true} />
        <LogDirectory grouped={true} />
      </SettingsGroup>

      <SettingsGroup title={t("settings.advanced.groups.experimental")}>
        <ExperimentalToggle grouped={true} />
        {experimentalEnabled && <LazyStreamClose grouped={true} />}
      </SettingsGroup>
    </SettingsPage>
  );
};
