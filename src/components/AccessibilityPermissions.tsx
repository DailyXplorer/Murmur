import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { checkAccessibilityPermission } from "tauri-plugin-macos-permissions-api";
import { CircleNotchIcon } from "@phosphor-icons/react/dist/csr/CircleNotch";
import { repairAccessibilityPermission } from "@/lib/macosPermissions";
import { Button } from "./ui/Button";

type PermissionState = "request" | "waiting" | "granted";

const AccessibilityPermissions: React.FC = () => {
  const { t } = useTranslation();
  const [hasAccessibility, setHasAccessibility] = useState<boolean>(false);
  const [permissionState, setPermissionState] =
    useState<PermissionState>("request");
  const [isRepairing, setIsRepairing] = useState(false);

  const handleButtonClick = async (): Promise<void> => {
    if (isRepairing || permissionState === "granted") return;

    setIsRepairing(true);
    setPermissionState("waiting");
    try {
      await repairAccessibilityPermission();
    } catch (error) {
      console.error("Error repairing Accessibility permission:", error);
      setPermissionState("request");
    } finally {
      setIsRepairing(false);
    }
  };

  useEffect(() => {
    let isMounted = true;

    const initialSetup = async (): Promise<void> => {
      try {
        const hasPermissions = await checkAccessibilityPermission();
        if (hasPermissions && isMounted) {
          setHasAccessibility(true);
          setPermissionState("granted");
        }
      } catch (error) {
        console.error("Error checking Accessibility permission:", error);
      }
    };

    void initialSetup();

    return () => {
      isMounted = false;
    };
  }, []);

  useEffect(() => {
    if (permissionState !== "waiting") return;

    const refreshPermission = async (): Promise<void> => {
      try {
        const hasPermissions = await checkAccessibilityPermission();
        if (hasPermissions) {
          setHasAccessibility(true);
          setPermissionState("granted");
        }
      } catch (error) {
        console.error("Error checking Accessibility permission:", error);
      }
    };

    const interval = setInterval(() => void refreshPermission(), 1000);

    return () => clearInterval(interval);
  }, [permissionState]);

  if (hasAccessibility) {
    return null;
  }

  return (
    <div className="p-4 w-full rounded-lg border border-mid-gray">
      <div className="flex justify-between items-center gap-2">
        <div className="">
          <p className="text-sm font-medium">
            {t("accessibility.permissionsDescription")}
          </p>
        </div>
        {permissionState === "waiting" ? (
          <div
            aria-live="polite"
            className="flex flex-wrap items-center justify-end gap-2 rounded-lg bg-logo-primary/10 px-3 py-2 text-sm"
          >
            <span className="flex min-h-control items-center gap-2">
              <CircleNotchIcon size={15} className="animate-spin shrink-0" />
              {t("onboarding.permissions.accessibility.waiting")}
            </span>
            <button
              type="button"
              disabled={isRepairing}
              onClick={handleButtonClick}
              className="min-h-control rounded-md px-2 font-medium underline underline-offset-2 hover:text-logo-primary disabled:cursor-wait disabled:opacity-50 transition-colors"
            >
              {t("accessibility.openSettings")}
            </button>
          </div>
        ) : permissionState === "request" ? (
          <Button onClick={handleButtonClick}>
            {t("onboarding.permissions.accessibility.action")}
          </Button>
        ) : null}
      </div>
    </div>
  );
};

export default AccessibilityPermissions;
