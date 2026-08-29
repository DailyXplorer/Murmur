import { commands } from "@/bindings";

/**
 * Replace a stale Accessibility entry and open the matching macOS privacy pane.
 * The backend scopes the reset to the running bundle identifier.
 */
export const repairAccessibilityPermission = async (): Promise<void> => {
  const result = await commands.repairAccessibilityPermission();

  if (result.status === "error") {
    throw new Error(result.error);
  }
};
