import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { type } from "@tauri-apps/plugin-os";
import { checkAccessibilityPermission } from "tauri-plugin-macos-permissions-api";
import { CircleNotchIcon } from "@phosphor-icons/react/dist/csr/CircleNotch";
import { repairAccessibilityPermission } from "@/lib/macosPermissions";

// Define permission state type
type PermissionState = "request" | "waiting" | "granted";

// Define button configuration type
interface ButtonConfig {
  className: string;
}

const AccessibilityPermissions: React.FC = () => {
  const { t } = useTranslation();
  const [hasAccessibility, setHasAccessibility] = useState<boolean>(false);
  const [permissionState, setPermissionState] =
    useState<PermissionState>("request");
  const [isRepairing, setIsRepairing] = useState(false);

  // Accessibility permissions are only required on macOS
  const isMacOS = type() === "macos";

  // Handle the unified button action based on current state
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

  // On app boot - check permissions (only on macOS)
  useEffect(() => {
    if (!isMacOS) return;

    const initialSetup = async (): Promise<void> => {
      const hasPermissions: boolean = await checkAccessibilityPermission();
      setHasAccessibility(hasPermissions);
      setPermissionState(hasPermissions ? "granted" : "request");
    };

    initialSetup();
  }, [isMacOS]);

  useEffect(() => {
    if (!isMacOS || permissionState !== "waiting") return;

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
  }, [isMacOS, permissionState]);

  // Skip rendering on non-macOS platforms or if permission is already granted
  if (!isMacOS || hasAccessibility) {
    return null;
  }

  // Configure button text and style based on state
  const buttonConfig: Record<PermissionState, ButtonConfig | null> = {
    request: {
      className:
        "px-3 py-2 min-h-10 text-sm font-medium bg-mid-gray/10 border border-mid-gray/80 hover:bg-logo-primary/10 rounded-lg cursor-pointer hover:border-logo-primary active:scale-[0.96] transition-[background-color,border-color,transform]",
    },
    waiting: null,
    granted: null,
  };

  const config = buttonConfig[permissionState];

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
            <span className="flex min-h-10 items-center gap-2">
              <CircleNotchIcon size={15} className="animate-spin shrink-0" />
              {t("onboarding.permissions.accessibility.waiting")}
            </span>
            <button
              type="button"
              disabled={isRepairing}
              onClick={handleButtonClick}
              className="min-h-10 rounded-md px-2 font-medium underline underline-offset-2 hover:text-logo-primary disabled:cursor-wait disabled:opacity-50 transition-colors"
            >
              {t("accessibility.openSettings")}
            </button>
          </div>
        ) : config ? (
          <button onClick={handleButtonClick} className={config.className}>
            {t("onboarding.permissions.accessibility.action")}
          </button>
        ) : null}
      </div>
    </div>
  );
};

export default AccessibilityPermissions;
