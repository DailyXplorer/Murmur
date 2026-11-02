import React from "react";
import { useTranslation } from "react-i18next";
import type { CallApp, CallPolicy } from "@/bindings";
import { useSettings } from "../../hooks/useSettings";
import { Dropdown } from "../ui/Dropdown";
import { SettingContainer } from "../ui/SettingContainer";
import { SettingsGroup } from "../ui/SettingsGroup";
import { useCallDetection } from "./MeetingCallPrompt";

const APPS: CallApp[] = [
  "zoom",
  "google_meet",
  "teams",
  "slack",
  "face_time",
  "webex",
  "discord",
];
const POLICIES: CallPolicy[] = ["ask", "always", "never"];

/** What each call app does when it starts a call. Hidden with the switch
 * that turns call prompts off, which turns the rules off too. */
export const MeetingCallRules: React.FC = () => {
  const { t } = useTranslation();
  const { getSetting, updateSetting, isUpdating } = useSettings();
  const detected = useCallDetection();

  if (!detected || !(getSetting("meeting_call_prompt") ?? true)) return null;
  const policies = getSetting("meeting_call_policies") ?? {};
  return (
    <SettingsGroup title={t("settings.advanced.meetingCallRules.title")}>
      {APPS.map((app) => {
        const name = t(`settings.advanced.meetingCallRules.apps.${app}`);
        return (
          <SettingContainer
            key={app}
            title={name}
            description={t("settings.advanced.meetingCallRules.description", {
              app: name,
            })}
            grouped={true}
          >
            <Dropdown
              options={POLICIES.map((value) => ({
                value,
                label: t(`settings.advanced.meetingCallRules.options.${value}`),
              }))}
              selectedValue={policies[app] ?? "ask"}
              onSelect={(value) =>
                void updateSetting("meeting_call_policies", {
                  ...policies,
                  [app]: value as CallPolicy,
                })
              }
              disabled={isUpdating("meeting_call_policies")}
            />
          </SettingContainer>
        );
      })}
    </SettingsGroup>
  );
};
