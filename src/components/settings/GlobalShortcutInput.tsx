import React, { useEffect, useState, useRef } from "react";
import { useTranslation } from "react-i18next";
import {
  getKeyName,
  formatKeyCombination,
  normalizeKey,
} from "../../lib/utils/keyboard";
import { ResetButton } from "../ui/ResetButton";
import {
  FIELD_SIZE_CLASSES,
  FIELD_SURFACE_CLASSES,
} from "../ui/controlClasses";
import { SettingContainer } from "../ui/SettingContainer";
import { useSettings } from "../../hooks/useSettings";
import { useShortcutCaptureStore } from "../../stores/shortcutCaptureStore";
import { commands } from "@/bindings";
import { listen } from "@tauri-apps/api/event";
import { toast } from "sonner";

const MODIFIER_KEYS = new Set(["ctrl", "option", "shift", "command"]);

// Read from the event flags rather than tracked keydowns, so a key whose
// keyup was swallowed can never linger in the recorded shortcut.
const modifiersHeld = (e: KeyboardEvent): string[] =>
  [
    e.ctrlKey && "ctrl",
    e.altKey && "option",
    e.shiftKey && "shift",
    e.metaKey && "command",
  ].filter((modifier): modifier is string => Boolean(modifier));

interface GlobalShortcutInputProps {
  descriptionMode?: "inline" | "tooltip";
  grouped?: boolean;
  shortcutId: string;
  disabled?: boolean;
}

export const GlobalShortcutInput: React.FC<GlobalShortcutInputProps> = ({
  descriptionMode = "tooltip",
  grouped = false,
  shortcutId,
  disabled = false,
}) => {
  const { t } = useTranslation();
  const { getSetting, updateBinding, resetBinding, isUpdating, isLoading } =
    useSettings();
  const [heldModifiers, setHeldModifiers] = useState<string[]>([]);
  const {
    capturingId,
    begin: beginCapture,
    end: endCapture,
  } = useShortcutCaptureStore();
  const editingShortcutId = capturingId === shortcutId ? shortcutId : null;
  const [originalBinding, setOriginalBinding] = useState<string>("");
  const [registrationFailed, setRegistrationFailed] = useState(false);
  const shortcutRefs = useRef<Map<string, HTMLElement | null>>(new Map());

  const bindings = getSetting("bindings") || {};

  useEffect(() => {
    let active = true;
    const refresh = () =>
      commands
        .getShortcutRegistrationErrors()
        .then((errors) => {
          if (active) setRegistrationFailed(errors[shortcutId] !== undefined);
        })
        .catch(() => undefined);
    void refresh();
    const unlisten = listen("shortcut-registration-errors-changed", refresh);
    return () => {
      active = false;
      void unlisten.then((fn) => fn());
    };
  }, [shortcutId]);

  useEffect(() => () => endCapture(shortcutId), [shortcutId, endCapture]);

  useEffect(() => {
    // Only add event listeners when we're in editing mode
    if (editingShortcutId === null) return;

    let cleanup = false;

    const cancelRecording = () => {
      endCapture(editingShortcutId);
      setHeldModifiers([]);
      setOriginalBinding("");
    };

    const commit = async (newShortcut: string) => {
      if (!bindings[editingShortcutId]) return;
      try {
        await updateBinding(editingShortcutId, newShortcut);
      } catch (error) {
        console.error("Failed to change binding:", error);
        toast.error(
          t("settings.general.shortcut.errors.set", {
            error: String(error),
          }),
        );

        // Reset to original binding on error
        if (originalBinding) {
          try {
            await updateBinding(editingShortcutId, originalBinding);
          } catch (resetError) {
            console.error("Failed to reset binding:", resetError);
            toast.error(t("settings.general.shortcut.errors.reset"));
          }
        }
      }

      // Re-registers all bindings (the one just committed is already
      // registered; re-registering it fails cleanly and is ignored)
      endCapture(editingShortcutId);
      setHeldModifiers([]);
      setOriginalBinding("");
    };

    // The shortcut is committed on the main key's keydown, as native macOS
    // recorders do: while Command is held, WebKit never delivers the other
    // keys' keyup, so waiting for every key to be released never ends.
    const handleKeyDown = (e: KeyboardEvent) => {
      if (cleanup) return;
      if (e.repeat) return; // ignore auto-repeat
      e.preventDefault();

      const key = normalizeKey(getKeyName(e));
      const modifiers = modifiersHeld(e);
      if (MODIFIER_KEYS.has(key)) {
        setHeldModifiers(modifiers);
        return;
      }

      cleanup = true;
      // A bare Escape abandons the edit, as in native macOS shortcut
      // recorders, instead of becoming a global key taken from every app.
      if (key === "esc" && modifiers.length === 0) {
        cancelRecording();
        return;
      }
      void commit([...modifiers, key].join("+"));
    };

    const handleKeyUp = (e: KeyboardEvent) => {
      if (cleanup) return;
      e.preventDefault();
      setHeldModifiers(modifiersHeld(e));
    };

    // Add click outside handler
    const handleClickOutside = async (e: MouseEvent) => {
      if (cleanup) return;
      const activeElement = shortcutRefs.current.get(editingShortcutId);
      if (activeElement && !activeElement.contains(e.target as Node)) {
        cancelRecording();
      }
    };

    window.addEventListener("keydown", handleKeyDown);
    window.addEventListener("keyup", handleKeyUp);
    window.addEventListener("click", handleClickOutside);

    return () => {
      cleanup = true;
      window.removeEventListener("keydown", handleKeyDown);
      window.removeEventListener("keyup", handleKeyUp);
      window.removeEventListener("click", handleClickOutside);
    };
  }, [editingShortcutId, bindings, originalBinding, updateBinding, endCapture]);

  // Start recording a new shortcut
  const startRecording = (id: string) => {
    if (editingShortcutId === id) return; // Already editing this shortcut

    // Store the original binding to restore if the change fails
    setOriginalBinding(bindings[id]?.current_binding || "");
    setHeldModifiers([]);
    // Suspends all bindings so no shortcut fires (or swallows the
    // keystrokes) while keys are being recorded
    beginCapture(id);
  };

  // Format the current shortcut keys being recorded
  const formatCurrentKeys = (): string => {
    if (heldModifiers.length === 0)
      return t("settings.general.shortcut.pressKeys");

    // Use the same formatting as the display to ensure consistency
    return formatKeyCombination(heldModifiers.join("+"));
  };

  // Store references to shortcut elements
  const setShortcutRef = (id: string, ref: HTMLElement | null) => {
    shortcutRefs.current.set(id, ref);
  };

  // If still loading, show loading state
  if (isLoading) {
    return (
      <SettingContainer
        title={t("settings.general.shortcut.title")}
        description={t("settings.general.shortcut.description")}
        descriptionMode={descriptionMode}
        grouped={grouped}
      >
        <div className="text-sm text-mid-gray">
          {t("settings.general.shortcut.loading")}
        </div>
      </SettingContainer>
    );
  }

  // If no bindings are loaded, show empty state
  if (Object.keys(bindings).length === 0) {
    return (
      <SettingContainer
        title={t("settings.general.shortcut.title")}
        description={t("settings.general.shortcut.description")}
        descriptionMode={descriptionMode}
        grouped={grouped}
      >
        <div className="text-sm text-mid-gray">
          {t("settings.general.shortcut.none")}
        </div>
      </SettingContainer>
    );
  }

  const binding = bindings[shortcutId];
  if (!binding) {
    return (
      <SettingContainer
        title={t("settings.general.shortcut.title")}
        description={t("settings.general.shortcut.notFound")}
        descriptionMode={descriptionMode}
        grouped={grouped}
      >
        <div className="text-sm text-mid-gray">
          {t("settings.general.shortcut.none")}
        </div>
      </SettingContainer>
    );
  }

  // Get translated name and description for the binding
  const translatedName = t(
    `settings.general.shortcut.bindings.${shortcutId}.name`,
    binding.name,
  );
  const translatedDescription = t(
    `settings.general.shortcut.bindings.${shortcutId}.description`,
    binding.description,
  );

  return (
    <SettingContainer
      title={translatedName}
      description={translatedDescription}
      descriptionMode={descriptionMode}
      grouped={grouped}
      disabled={disabled}
      layout="horizontal"
    >
      <div className="flex min-w-0 flex-col gap-1">
        <div className="flex min-w-0 items-center space-x-1">
          {editingShortcutId === shortcutId ? (
            <div
              ref={(ref) => setShortcutRef(shortcutId, ref)}
              role="status"
              aria-live="polite"
              data-control="field"
              className={`${FIELD_SIZE_CLASSES} flex min-w-0 flex-1 items-center border-logo-primary bg-logo-primary/30`}
            >
              <span className="truncate">{formatCurrentKeys()}</span>
            </div>
          ) : (
            <button
              type="button"
              aria-label={`${translatedName}: ${t("settings.general.shortcut.pressKeys")}`}
              disabled={disabled || isUpdating(`binding_${shortcutId}`)}
              data-control="field"
              className={`${FIELD_SIZE_CLASSES} ${FIELD_SURFACE_CLASSES} min-w-0 flex-1 cursor-pointer truncate hover:border-logo-primary hover:bg-logo-primary/10 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-logo-primary disabled:cursor-not-allowed disabled:opacity-50`}
              onClick={(event) => {
                event.stopPropagation();
                startRecording(shortcutId);
              }}
            >
              {formatKeyCombination(binding.current_binding)}
            </button>
          )}
          <ResetButton
            className="shrink-0"
            onClick={() => resetBinding(shortcutId)}
            disabled={isUpdating(`binding_${shortcutId}`)}
            ariaLabel={`${t("common.reset")}: ${translatedName}`}
          />
        </div>
        {registrationFailed && (
          <p
            role="alert"
            className="text-end text-xs text-red-600 dark:text-red-400"
          >
            {t("settings.general.shortcut.errors.registrationFailed")}
          </p>
        )}
      </div>
    </SettingContainer>
  );
};
