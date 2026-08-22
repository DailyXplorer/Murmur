import React, { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { commands } from "@/bindings";
import { SettingsGroup } from "../../ui/SettingsGroup";
import { SettingContainer } from "../../ui/SettingContainer";

export const TranscriptionSettings: React.FC = () => {
  const { t } = useTranslation();
  const [signedIn, setSignedIn] = useState<boolean | null>(null);

  useEffect(() => {
    let cancelled = false;
    commands
      .getCodexAuthStatus()
      .then((status) => {
        if (!cancelled) setSignedIn(status.signed_in);
      })
      .catch(() => {
        if (!cancelled) setSignedIn(false);
      });
    return () => {
      cancelled = true;
    };
  }, []);

  return (
    <div className="max-w-3xl w-full mx-auto space-y-6">
      <SettingsGroup title={t("settings.transcription.title")}>
        <SettingContainer
          title={t("settings.transcription.sessionTitle")}
          description={t("settings.transcription.sessionDescription")}
          grouped={true}
        >
          <span className="text-sm text-text/80">
            {signedIn === null
              ? t("settings.transcription.checking")
              : signedIn
                ? t("settings.transcription.ready")
                : t("settings.transcription.missing")}
          </span>
        </SettingContainer>
      </SettingsGroup>
    </div>
  );
};
