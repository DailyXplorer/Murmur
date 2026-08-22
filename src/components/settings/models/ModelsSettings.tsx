import React, { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { commands } from "@/bindings";
import { SettingsGroup } from "../../ui/SettingsGroup";
import { SettingContainer } from "../../ui/SettingContainer";

export const ModelsSettings: React.FC = () => {
  const { t } = useTranslation();
  const [signedIn, setSignedIn] = useState<boolean | null>(null);

  useEffect(() => {
    let cancelled = false;
    commands
      .getCodexAuthStatus()
      .then((status) => {
        if (!cancelled) {
          setSignedIn(status.signed_in);
        }
      })
      .catch(() => {
        if (!cancelled) {
          setSignedIn(false);
        }
      });
    return () => {
      cancelled = true;
    };
  }, []);

  return (
    <div className="max-w-3xl w-full mx-auto space-y-6">
      <SettingsGroup title={t("settings.models.title")}>
        <SettingContainer
          title={t("settings.models.codexTitle")}
          description={t("settings.models.codexDescription")}
          grouped={true}
        >
          <span className="text-sm text-text/80">
            {signedIn === null
              ? t("settings.models.codexChecking")
              : signedIn
                ? t("settings.models.codexReady")
                : t("settings.models.codexMissing")}
          </span>
        </SettingContainer>
      </SettingsGroup>
    </div>
  );
};
