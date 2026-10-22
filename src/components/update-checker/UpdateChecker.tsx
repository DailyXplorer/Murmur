import React, { useCallback, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { check } from "@tauri-apps/plugin-updater";
import { listen } from "@tauri-apps/api/event";
import { toast } from "sonner";
import { commands, events } from "@/bindings";
import { ProgressBar } from "../shared";
import { useSettings } from "../../hooks/useSettings";

interface UpdateCheckerProps {
  className?: string;
}

const UpdateChecker: React.FC<UpdateCheckerProps> = ({ className = "" }) => {
  const { t } = useTranslation();
  const [isChecking, setIsChecking] = useState(false);
  const [updateAvailable, setUpdateAvailable] = useState(false);
  const [isInstalling, setIsInstalling] = useState(false);
  const [downloadProgress, setDownloadProgress] = useState(0);
  const [showUpToDate, setShowUpToDate] = useState(false);

  const { settings, isLoading } = useSettings();
  const settingsLoaded = !isLoading && settings !== null;
  const updateChecksEnabled = settings?.update_checks_enabled ?? false;

  const upToDateTimeoutRef = useRef<ReturnType<typeof setTimeout>>(undefined);
  const isCheckingRef = useRef(false);
  const downloadedBytesRef = useRef(0);
  const contentLengthRef = useRef(0);

  // The backend checks on its own schedule and owns the result.
  useEffect(() => {
    let heard = false;
    const unlisten = events.updateStatus.listen(({ payload }) => {
      heard = true;
      setUpdateAvailable(payload.available !== null);
    });
    void commands.getUpdateStatus().then(({ available }) => {
      if (!heard) setUpdateAvailable(available !== null);
    });
    return () => {
      void unlisten.then((fn) => fn());
    };
  }, []);

  const handleManualUpdateCheck = useCallback(async () => {
    if (!updateChecksEnabled || isCheckingRef.current) return;
    isCheckingRef.current = true;
    setIsChecking(true);
    const result = await commands.checkForUpdates();
    isCheckingRef.current = false;
    setIsChecking(false);

    if (result.status === "error") {
      toast.error(t("footer.checkForUpdates"), { description: result.error });
      return;
    }
    if (result.data.available === null) {
      setShowUpToDate(true);
      if (upToDateTimeoutRef.current) {
        clearTimeout(upToDateTimeoutRef.current);
      }
      upToDateTimeoutRef.current = setTimeout(() => {
        setShowUpToDate(false);
      }, 3000);
    }
  }, [t, updateChecksEnabled]);

  useEffect(() => {
    if (!settingsLoaded) return;

    if (!updateChecksEnabled) {
      if (upToDateTimeoutRef.current) {
        clearTimeout(upToDateTimeoutRef.current);
      }
      setShowUpToDate(false);
      return;
    }

    const updateUnlisten = listen("check-for-updates", () => {
      void handleManualUpdateCheck();
    });

    return () => {
      if (upToDateTimeoutRef.current) {
        clearTimeout(upToDateTimeoutRef.current);
      }
      updateUnlisten.then((fn) => fn());
    };
  }, [handleManualUpdateCheck, settingsLoaded, updateChecksEnabled]);

  const installUpdate = async () => {
    if (!updateChecksEnabled) return;

    try {
      setIsInstalling(true);
      setDownloadProgress(0);
      downloadedBytesRef.current = 0;
      contentLengthRef.current = 0;
      const update = await check();

      if (!update) {
        console.log("No update available during install attempt");
        return;
      }

      await update.downloadAndInstall((event) => {
        switch (event.event) {
          case "Started":
            downloadedBytesRef.current = 0;
            contentLengthRef.current = event.data.contentLength ?? 0;
            break;
          case "Progress":
            downloadedBytesRef.current += event.data.chunkLength;
            const progress =
              contentLengthRef.current > 0
                ? Math.round(
                    (downloadedBytesRef.current / contentLengthRef.current) *
                      100,
                  )
                : 0;
            setDownloadProgress(Math.min(progress, 100));
            break;
        }
      });
      const relaunched = await commands.relaunchAfterUpdate();
      if (relaunched.status === "error") throw new Error(relaunched.error);
    } catch (error) {
      console.error("Failed to install update:", error);
      toast.error(t("footer.updateAvailableShort"), {
        description: error instanceof Error ? error.message : String(error),
      });
    } finally {
      setIsInstalling(false);
      setDownloadProgress(0);
      downloadedBytesRef.current = 0;
      contentLengthRef.current = 0;
    }
  };

  const getUpdateStatusText = () => {
    if (!updateChecksEnabled) {
      return t("footer.updateCheckingDisabled");
    }
    if (isInstalling) {
      return downloadProgress > 0 && downloadProgress < 100
        ? t("footer.downloading", {
            progress: downloadProgress.toString().padStart(3),
          })
        : downloadProgress === 100
          ? t("footer.installing")
          : t("footer.preparing");
    }
    if (isChecking) return t("footer.checkingUpdates");
    if (showUpToDate) return t("footer.upToDate");
    if (updateAvailable) return t("footer.updateAvailableShort");
    return t("footer.checkForUpdates");
  };

  const getUpdateStatusAction = () => {
    if (!updateChecksEnabled) return undefined;
    if (updateAvailable && !isInstalling) return installUpdate;
    if (!isChecking && !isInstalling && !updateAvailable)
      return handleManualUpdateCheck;
    return undefined;
  };

  const isUpdateDisabled = !updateChecksEnabled || isChecking || isInstalling;
  const isUpdateClickable =
    !isUpdateDisabled && (updateAvailable || (!isChecking && !showUpToDate));

  return (
    <div className={`flex items-center gap-3 ${className}`}>
      {isUpdateClickable ? (
        <button
          onClick={getUpdateStatusAction()}
          disabled={isUpdateDisabled}
          className={`transition-colors disabled:opacity-50 tabular-nums ${
            updateAvailable
              ? "-my-0.5 rounded-full bg-background-ui px-2.5 py-0.5 font-medium text-on-accent hover:bg-background-ui-hover active:bg-background-ui-active focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent-text focus-visible:ring-offset-2 focus-visible:ring-offset-background"
              : "text-text/60 hover:text-text/80"
          }`}
        >
          {getUpdateStatusText()}
        </button>
      ) : (
        <span className="text-text/60 tabular-nums">
          {getUpdateStatusText()}
        </span>
      )}

      {isInstalling && downloadProgress > 0 && downloadProgress < 100 && (
        <ProgressBar percentage={downloadProgress} size="large" />
      )}
    </div>
  );
};

export default UpdateChecker;
