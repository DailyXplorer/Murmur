import { create } from "zustand";
import { subscribeWithSelector } from "zustand/middleware";
import { listen } from "@tauri-apps/api/event";
import type {
  AppSettings as Settings,
  AudioDevice,
  Result,
  TranscriptionProvider,
} from "@/bindings";
import { commands } from "@/bindings";

interface SettingsStore {
  settings: Settings | null;
  defaultSettings: Settings | null;
  isLoading: boolean;
  isUpdating: Record<string, boolean>;
  audioDevices: AudioDevice[];
  outputDevices: AudioDevice[];
  customSounds: { start: boolean; stop: boolean };

  // Actions
  initialize: () => Promise<void>;
  loadDefaultSettings: () => Promise<void>;
  updateSetting: <K extends keyof Settings>(
    key: K,
    value: Settings[K],
  ) => Promise<boolean>;
  resetSetting: (key: keyof Settings) => Promise<void>;
  refreshSettings: () => Promise<void>;
  refreshAudioDevices: () => Promise<void>;
  refreshOutputDevices: () => Promise<void>;
  updateBinding: (id: string, binding: string) => Promise<void>;
  resetBinding: (id: string) => Promise<void>;
  getSetting: <K extends keyof Settings>(key: K) => Settings[K] | undefined;
  isUpdatingKey: (key: string) => boolean;
  playTestSound: (soundType: "start" | "stop") => Promise<void>;
  checkCustomSounds: () => Promise<void>;

  // Internal state setters
  setSettings: (settings: Settings | null) => void;
  setDefaultSettings: (defaultSettings: Settings | null) => void;
  setLoading: (loading: boolean) => void;
  setUpdating: (key: string, updating: boolean) => void;
  setAudioDevices: (devices: AudioDevice[]) => void;
  setOutputDevices: (devices: AudioDevice[]) => void;
  setCustomSounds: (sounds: { start: boolean; stop: boolean }) => void;
}

const DEFAULT_AUDIO_DEVICE: AudioDevice = {
  index: "default",
  name: "Default",
  is_default: true,
};

const SETTINGS_CHANGED_LISTENER_RETRY_DELAYS_MS = [250, 1000];

const previousSettingsChangedListenerCleanup: Promise<void> =
  import.meta.hot?.data.settingsChangedListenerCleanup ?? Promise.resolve();

type SettingsChangedEvent = { payload: { setting?: string } };
type SettingsChangedUnlisten = () => void | Promise<void>;

interface SettingsChangedListenerLifecycleOptions {
  previousCleanup?: Promise<void>;
  listen: (
    handler: (event: SettingsChangedEvent) => void,
  ) => Promise<SettingsChangedUnlisten>;
  onSettingsChanged: (event: SettingsChangedEvent) => void;
}

export const createSettingsChangedListenerLifecycle = ({
  previousCleanup = Promise.resolve(),
  listen: registerListener,
  onSettingsChanged,
}: SettingsChangedListenerLifecycleOptions) => {
  let registered = false;
  let unlisten: SettingsChangedUnlisten | null = null;
  let registration: Promise<void> | null = null;
  let disposed = false;
  let retryTimer: number | null = null;
  let registrationAttempts = 0;

  const unregister = async (listenerUnlisten: SettingsChangedUnlisten) => {
    try {
      await listenerUnlisten();
    } catch (error) {
      console.error("Failed to unregister settings listener:", error);
    }
  };

  const clearRetry = () => {
    if (retryTimer !== null) {
      window.clearTimeout(retryTimer);
      retryTimer = null;
    }
  };

  const scheduleRetry = () => {
    if (disposed || registered || retryTimer !== null) {
      return;
    }

    const delay =
      SETTINGS_CHANGED_LISTENER_RETRY_DELAYS_MS[registrationAttempts - 1];
    if (delay === undefined) {
      return;
    }

    retryTimer = window.setTimeout(() => {
      retryTimer = null;
      void register();
    }, delay);
  };

  const register = () => {
    if (disposed || registered) {
      return Promise.resolve();
    }

    if (registration) {
      return registration;
    }

    registrationAttempts += 1;
    const activeRegistration = (async () => {
      try {
        const listenerUnlisten = await registerListener(onSettingsChanged);

        if (disposed) {
          await unregister(listenerUnlisten);
          return;
        }

        unlisten = listenerUnlisten;
        registered = true;
        registrationAttempts = 0;
      } catch (error) {
        console.error("Failed to register settings listener:", error);
        scheduleRetry();
      }
    })();

    registration = activeRegistration;
    void activeRegistration.finally(() => {
      if (registration === activeRegistration) {
        registration = null;
      }
    });

    return activeRegistration;
  };

  const initialize = async () => {
    await previousCleanup;
    if (disposed) {
      return;
    }
    await register();
  };

  const dispose = async () => {
    disposed = true;
    clearRetry();

    await previousCleanup;

    if (registration) {
      await registration;
    }

    clearRetry();

    const listenerUnlisten = unlisten;
    unlisten = null;
    registered = false;

    if (listenerUnlisten) {
      await unregister(listenerUnlisten);
    }
  };

  return { dispose, initialize };
};

let initializationPromise: Promise<void> | null = null;

const settingsChangedListenerLifecycle = createSettingsChangedListenerLifecycle(
  {
    previousCleanup: previousSettingsChangedListenerCleanup,
    listen: (handler) =>
      listen<{ setting?: string }>("settings-changed", handler),
    onSettingsChanged: (event) => {
      useSettingsStore.getState().refreshSettings();
      if (event.payload.setting === "selected_microphone") {
        useSettingsStore.getState().refreshAudioDevices();
      }
    },
  },
);

if (import.meta.hot) {
  import.meta.hot.dispose((data) => {
    data.settingsChangedListenerCleanup =
      settingsChangedListenerLifecycle.dispose();
  });
}

const settingUpdaters: {
  [K in keyof Settings]?: (
    value: Settings[K],
  ) => Promise<Result<unknown, string>>;
} = {
  always_on_microphone: (value) =>
    commands.updateMicrophoneMode(value as boolean),
  audio_feedback: (value) =>
    commands.changeAudioFeedbackSetting(value as boolean),
  audio_feedback_volume: (value) =>
    commands.changeAudioFeedbackVolumeSetting(value as number),
  sound_theme: (value) => commands.changeSoundThemeSetting(value as string),
  start_hidden: (value) => commands.changeStartHiddenSetting(value as boolean),
  autostart_enabled: (value) =>
    commands.changeAutostartSetting(value as boolean),
  update_checks_enabled: (value) =>
    commands.changeUpdateChecksSetting(value as boolean),
  show_whats_new_on_update: (value) =>
    commands.changeShowWhatsNewOnUpdateSetting(value as boolean),
  whats_new_last_seen_version: (value) =>
    commands.changeWhatsNewLastSeenVersionSetting(value as string),
  push_to_talk: (value) => commands.changePttSetting(value as boolean),
  selected_microphone: (value) =>
    commands.setSelectedMicrophone(
      (value as string) === "Default" || value === null
        ? "default"
        : (value as string),
    ),
  selected_channel: (value) =>
    commands.setSelectedChannel((value as number | null | undefined) ?? null),
  clamshell_microphone: (value) =>
    commands.setClamshellMicrophone(
      (value as string) === "Default" ? "default" : (value as string),
    ),
  selected_output_device: (value) =>
    commands.setSelectedOutputDevice(
      (value as string) === "Default" || value === null
        ? "default"
        : (value as string),
    ),
  recording_retention_period: (value) =>
    commands.updateRecordingRetentionPeriod(value as string),
  selected_language: (value) =>
    commands.changeSelectedLanguageSetting(value as string),
  transcription_provider: (value) =>
    commands.changeTranscriptionProviderSetting(value as TranscriptionProvider),
  overlay_position: (value) =>
    commands.changeOverlayPositionSetting(value as string),
  debug_mode: (value) => commands.changeDebugModeSetting(value as boolean),
  custom_words: (value) => commands.updateCustomWords(value as string[]),
  word_correction_threshold: (value) =>
    commands.changeWordCorrectionThresholdSetting(value as number),
  paste_delay_ms: (value) =>
    commands.changePasteDelayMsSetting(value as number),
  paste_delay_after_ms: (value) =>
    commands.changePasteDelayAfterMsSetting(value as number),
  reliable_paste: (value) =>
    commands.changeReliablePasteSetting(value as boolean),
  paste_method: (value) => commands.changePasteMethodSetting(value as string),
  clipboard_handling: (value) =>
    commands.changeClipboardHandlingSetting(value as string),
  auto_submit: (value) => commands.changeAutoSubmitSetting(value as boolean),
  auto_submit_key: (value) =>
    commands.changeAutoSubmitKeySetting(value as string),
  history_limit: (value) => commands.updateHistoryLimit(value as number),
  mute_while_recording: (value) =>
    commands.changeMuteWhileRecordingSetting(value as boolean),
  append_trailing_space: (value) =>
    commands.changeAppendTrailingSpaceSetting(value as boolean),
  log_level: (value) => commands.setLogLevel(value as any),
  app_language: (value) => commands.changeAppLanguageSetting(value as string),
  theme: (value) => commands.changeThemeSetting(value as string),
  accent_color: (value) => commands.changeAccentColorSetting(value as string),
  experimental_enabled: (value) =>
    commands.changeExperimentalEnabledSetting(value as boolean),
  lazy_stream_close: (value) =>
    commands.changeLazyStreamCloseSetting(value as boolean),
  overlay_style: (value) => commands.changeOverlayStyleSetting(value as string),
  filler_word_removal_enabled: (value) =>
    commands.changeFillerWordRemovalEnabledSetting(value as boolean),
  show_tray_icon: (value) =>
    commands.changeShowTrayIconSetting(value as boolean),
  extra_recording_buffer_ms: (value) =>
    commands.changeExtraRecordingBufferSetting(value as number),
};

interface SettingWriteState {
  committedValue: Settings[keyof Settings];
  latestRevision: number;
  pendingCount: number;
  tail: Promise<void>;
}

const MAX_SETTINGS_REFRESH_ATTEMPTS = 3;
const settingWriteStates = new Map<keyof Settings, SettingWriteState>();
let settingsRefreshRevision = 0;
let settingsWriteRevision = 0;

export const useSettingsStore = create<SettingsStore>()(
  subscribeWithSelector((set, get) => ({
    settings: null,
    defaultSettings: null,
    isLoading: true,
    isUpdating: {},
    audioDevices: [],
    outputDevices: [],
    customSounds: { start: false, stop: false },

    // Internal setters
    setSettings: (settings) => set({ settings }),
    setDefaultSettings: (defaultSettings) => set({ defaultSettings }),
    setLoading: (isLoading) => set({ isLoading }),
    setUpdating: (key, updating) =>
      set((state) => ({
        isUpdating: { ...state.isUpdating, [key]: updating },
      })),
    setAudioDevices: (audioDevices) => set({ audioDevices }),
    setOutputDevices: (outputDevices) => set({ outputDevices }),
    setCustomSounds: (customSounds) => set({ customSounds }),

    // Getters
    getSetting: (key) => get().settings?.[key],
    isUpdatingKey: (key) => get().isUpdating[key] || false,

    // Load settings from store
    refreshSettings: async () => {
      const refreshRevision = ++settingsRefreshRevision;

      try {
        for (
          let attempt = 0;
          attempt < MAX_SETTINGS_REFRESH_ATTEMPTS;
          attempt++
        ) {
          const activeWrites = [...settingWriteStates.values()].map(
            (writeState) => writeState.tail,
          );
          await Promise.all(activeWrites);

          if (refreshRevision !== settingsRefreshRevision) return;
          if (settingWriteStates.size > 0) continue;

          const revisionBeforeRequest = settingsWriteRevision;
          const result = await commands.getAppSettings();
          if (refreshRevision !== settingsRefreshRevision) return;
          if (result.status === "error") {
            console.error("Failed to load settings:", result.error);
            set({ isLoading: false });
            return;
          }

          if (
            revisionBeforeRequest !== settingsWriteRevision ||
            settingWriteStates.size > 0
          ) {
            continue;
          }

          const settings = result.data;
          const normalizedSettings: Settings = {
            ...settings,
            always_on_microphone: settings.always_on_microphone ?? false,
            selected_microphone: settings.selected_microphone ?? "Default",
            clamshell_microphone: settings.clamshell_microphone ?? "Default",
            selected_output_device:
              settings.selected_output_device ?? "Default",
          };
          set({ settings: normalizedSettings, isLoading: false });
          return;
        }

        if (refreshRevision !== settingsRefreshRevision) return;
        console.warn("Skipped stale settings refresh while writes were active");
        set({ isLoading: false });
      } catch (error) {
        if (refreshRevision !== settingsRefreshRevision) return;
        console.error("Failed to load settings:", error);
        set({ isLoading: false });
      }
    },

    // Load audio devices
    refreshAudioDevices: async () => {
      try {
        const result = await commands.getAvailableMicrophones();
        if (result.status === "ok") {
          const devicesWithDefault = [
            DEFAULT_AUDIO_DEVICE,
            ...result.data.filter(
              (d) => d.name !== "Default" && d.name !== "default",
            ),
          ];
          set({ audioDevices: devicesWithDefault });
        } else {
          set({ audioDevices: [DEFAULT_AUDIO_DEVICE] });
        }
      } catch (error) {
        console.error("Failed to load audio devices:", error);
        set({ audioDevices: [DEFAULT_AUDIO_DEVICE] });
      }
    },

    // Load output devices
    refreshOutputDevices: async () => {
      try {
        const result = await commands.getAvailableOutputDevices();
        if (result.status === "ok") {
          const devicesWithDefault = [
            DEFAULT_AUDIO_DEVICE,
            ...result.data.filter(
              (d) => d.name !== "Default" && d.name !== "default",
            ),
          ];
          set({ outputDevices: devicesWithDefault });
        } else {
          set({ outputDevices: [DEFAULT_AUDIO_DEVICE] });
        }
      } catch (error) {
        console.error("Failed to load output devices:", error);
        set({ outputDevices: [DEFAULT_AUDIO_DEVICE] });
      }
    },

    // Play a test sound
    playTestSound: async (soundType: "start" | "stop") => {
      try {
        await commands.playTestSound(soundType);
      } catch (error) {
        console.error(`Failed to play test sound (${soundType}):`, error);
      }
    },

    checkCustomSounds: async () => {
      try {
        const sounds = await commands.checkCustomSounds();
        get().setCustomSounds(sounds);
      } catch (error) {
        console.error("Failed to check custom sounds:", error);
      }
    },

    // Update a specific setting
    updateSetting: async <K extends keyof Settings>(
      key: K,
      value: Settings[K],
    ) => {
      const { settings, setUpdating } = get();
      const updateKey = String(key);
      settingsWriteRevision += 1;
      let writeState = settingWriteStates.get(key);

      if (!writeState) {
        writeState = {
          committedValue: settings?.[key],
          latestRevision: 0,
          pendingCount: 0,
          tail: Promise.resolve(),
        };
        settingWriteStates.set(key, writeState);
      }

      const revision = writeState.latestRevision + 1;
      writeState.latestRevision = revision;
      writeState.pendingCount += 1;

      setUpdating(updateKey, true);
      set((state) => ({
        settings: state.settings ? { ...state.settings, [key]: value } : null,
      }));

      const operation = writeState.tail.then(async () => {
        try {
          const updater = settingUpdaters[key];
          if (!updater) {
            throw new Error(`No handler for setting: ${String(key)}`);
          }

          const result = await updater(value);
          if (result.status === "error") throw new Error(result.error);

          writeState.committedValue = value;
          return true;
        } catch (error) {
          console.error(`Failed to update setting ${String(key)}:`, error);
          if (
            revision === writeState.latestRevision &&
            Object.is(get().settings?.[key], value)
          ) {
            set((state) => ({
              settings: state.settings
                ? { ...state.settings, [key]: writeState.committedValue }
                : null,
            }));
          }
          return false;
        } finally {
          writeState.pendingCount -= 1;
          if (revision === writeState.latestRevision) {
            setUpdating(updateKey, false);
          }
          if (
            writeState.pendingCount === 0 &&
            settingWriteStates.get(key) === writeState
          ) {
            settingWriteStates.delete(key);
          }
        }
      });

      writeState.tail = operation.then(() => undefined);
      return operation;
    },

    // Reset a setting to its default value
    resetSetting: async (key) => {
      const { defaultSettings } = get();
      if (defaultSettings) {
        const defaultValue = defaultSettings[key];
        if (defaultValue !== undefined) {
          await get().updateSetting(key, defaultValue as any);
        }
      }
    },

    // Update a specific binding
    updateBinding: async (id, binding) => {
      const { settings, setUpdating } = get();
      const updateKey = `binding_${id}`;
      const originalBinding = settings?.bindings?.[id]?.current_binding;

      setUpdating(updateKey, true);

      try {
        // Optimistic update
        set((state) => ({
          settings: state.settings
            ? {
                ...state.settings,
                bindings: {
                  ...state.settings.bindings,
                  [id]: {
                    ...state.settings.bindings?.[id]!,
                    current_binding: binding,
                  },
                },
              }
            : null,
        }));

        const result = await commands.changeBinding(id, binding);

        // Check if the command executed successfully
        if (result.status === "error") {
          throw new Error(result.error);
        }

        // Check if the binding change was successful
        if (!result.data.success) {
          throw new Error(result.data.error || "Failed to update binding");
        }
      } catch (error) {
        console.error(`Failed to update binding ${id}:`, error);

        // Rollback on error
        if (originalBinding && get().settings) {
          set((state) => ({
            settings: state.settings
              ? {
                  ...state.settings,
                  bindings: {
                    ...state.settings.bindings,
                    [id]: {
                      ...state.settings.bindings?.[id]!,
                      current_binding: originalBinding,
                    },
                  },
                }
              : null,
          }));
        }

        // Re-throw to let the caller know it failed
        throw error;
      } finally {
        setUpdating(updateKey, false);
      }
    },

    // Reset a specific binding
    resetBinding: async (id) => {
      const { setUpdating, refreshSettings } = get();
      const updateKey = `binding_${id}`;

      setUpdating(updateKey, true);

      try {
        await commands.resetBinding(id);
        await refreshSettings();
      } catch (error) {
        console.error(`Failed to reset binding ${id}:`, error);
      } finally {
        setUpdating(updateKey, false);
      }
    },

    // Load default settings from Rust
    loadDefaultSettings: async () => {
      try {
        const result = await commands.getDefaultSettings();
        if (result.status === "ok") {
          set({ defaultSettings: result.data });
        } else {
          console.error("Failed to load default settings:", result.error);
        }
      } catch (error) {
        console.error("Failed to load default settings:", error);
      }
    },

    initialize: () => {
      if (initializationPromise) {
        return initializationPromise;
      }

      initializationPromise = (async () => {
        const { refreshSettings, checkCustomSounds, loadDefaultSettings } =
          get();

        // Note: Audio devices are NOT refreshed here. The frontend (App.tsx)
        // is responsible for calling refreshAudioDevices/refreshOutputDevices
        // after onboarding completes. This avoids triggering permission dialogs
        // on macOS before the user is ready.
        await Promise.all([
          loadDefaultSettings(),
          refreshSettings(),
          checkCustomSounds(),
        ]);

        await settingsChangedListenerLifecycle.initialize();
      })().finally(() => {
        initializationPromise = null;
      });

      return initializationPromise;
    },
  })),
);
