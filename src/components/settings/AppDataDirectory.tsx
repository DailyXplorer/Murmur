import React, { useState, useEffect } from "react";
import { useTranslation } from "react-i18next";
import { commands } from "@/bindings";
import { SettingContainer } from "../ui/SettingContainer";
import { PathDisplay } from "../ui/PathDisplay";

interface AppDataDirectoryProps {
  descriptionMode?: "tooltip" | "inline";
  grouped?: boolean;
}

export const AppDataDirectory: React.FC<AppDataDirectoryProps> = ({
  descriptionMode = "inline",
  grouped = false,
}) => {
  const { t } = useTranslation();
  const [appDirPath, setAppDirPath] = useState<string>("");
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    const loadAppDirectory = async () => {
      try {
        const result = await commands.getAppDirPath();
        if (result.status === "ok") {
          setAppDirPath(result.data);
        } else {
          setError(result.error);
        }
      } catch (err) {
        setError(
          err instanceof Error ? err.message : "Failed to load app directory",
        );
      } finally {
        setLoading(false);
      }
    };

    loadAppDirectory();
  }, []);

  const handleOpen = async () => {
    if (!appDirPath) return;
    try {
      await commands.openAppDataDir();
    } catch (openError) {
      console.error("Failed to open app data directory:", openError);
    }
  };

  return (
    <SettingContainer
      title={t("settings.about.appDataDirectory.title")}
      description={t("settings.about.appDataDirectory.description")}
      descriptionMode={descriptionMode}
      grouped={grouped}
      layout="stacked"
    >
      {loading ? (
        <div
          className="h-10 animate-pulse rounded-md bg-mid-gray/10"
          aria-hidden="true"
        />
      ) : error ? (
        <p
          role="alert"
          className="rounded-md border border-error/30 bg-error/10 p-3 text-sm text-error"
        >
          {t("errors.loadDirectory", { error })}
        </p>
      ) : (
        <PathDisplay
          path={appDirPath}
          onOpen={handleOpen}
          disabled={!appDirPath}
        />
      )}
    </SettingContainer>
  );
};
