import { emit } from "@tauri-apps/api/event";
import type {
  DictationStyle,
  Engine,
  LocalModelStatus,
} from "../../../src/bindings";
import { events } from "../../../src/bindings";
import { MODELS, modelsStatus } from "./models";
import { engineRefused, pendingMainActions, styleRefused } from "./params";
import { SETTINGS } from "./settings";

let modelNow = modelsStatus;
export const setModel = (status: LocalModelStatus) => {
  modelNow = status;
  void events.localModelStatus.emit(status);
};

/** The backend's switch to this Mac once the model is ready. */
export const switchIfAsked = () => {
  if (!SETTINGS.switch_to_local_when_ready || modelNow.state.state !== "ready")
    return;
  SETTINGS.switch_to_local_when_ready = false;
  SETTINGS.transcription_engine = "local";
  void emit("settings-changed", { setting: "transcription_engine" });
  pendingMainActions.add("switched_to_local");
  void events.switchedToLocal.emit({});
};

export const dropSwitch = () => {
  if (!SETTINGS.switch_to_local_when_ready) return;
  SETTINGS.switch_to_local_when_ready = false;
  void emit("settings-changed", { setting: "switch_to_local_when_ready" });
};

/** Engine and card commands, which change the stored settings. */
export const handleEngineCommand = (
  command: string,
  args: Record<string, unknown>,
): { handled: boolean; value?: unknown } => {
  switch (command) {
    case "get_app_settings":
      return { handled: true, value: structuredClone(SETTINGS) };
    case "get_local_model_status":
      return { handled: true, value: modelNow };
    case "change_transcription_engine_setting":
      // The backend refuses an engine this Mac cannot run.
      if (engineRefused) throw "This Mac cannot run the local model";
      SETTINGS.transcription_engine = args.engine as Engine;
      dropSwitch();
      if (args.engine === "local" && modelNow.state.state !== "ready") {
        setModel(MODELS.downloading);
      }
      return { handled: true, value: null };
    case "switch_to_local_when_ready":
      SETTINGS.switch_to_local_when_ready = true;
      if (modelNow.state.state === "ready") switchIfAsked();
      else setModel(MODELS.downloading);
      return { handled: true, value: null };
    case "change_dictation_style_setting":
      if (styleRefused) throw "The settings store is read-only";
      SETTINGS.dictation_style = args.style as DictationStyle;
      return { handled: true, value: null };
    case "change_cleanup_short_dictations_setting":
      SETTINGS.cleanup_short_dictations = args.enabled as boolean;
      return { handled: true, value: null };
    case "change_leading_space_setting":
      SETTINGS.leading_space = args.enabled === true;
      return { handled: true, value: null };
    case "change_local_stt_card_dismissed_setting":
      SETTINGS.local_stt_card_dismissed = args.dismissed as boolean;
      return { handled: true, value: null };
    default:
      return { handled: false };
  }
};
