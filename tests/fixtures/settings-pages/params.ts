import type {
  DictationStyle,
  Engine,
  MeetingStage,
  ProcessingError,
  SpeakerAnalysis,
} from "../../../src/bindings";

export type Keyboard = "azerty" | "qwerty";

export const params = new URLSearchParams(window.location.search);
export const language = params.get("lang") ?? "en";
export const showOnboarding = params.get("onboarding") === "1";
/** On by default, as in the app; `meetings=0` turns them off. */
export const meetingsEnabled = params.get("meetings") !== "0";
export const systemAudioSilent = params.get("systemAudioSilent") === "1";
export const systemAudioPermission =
  params.get("systemAudioPermission") ?? "granted";
export const audioBytes = Number(params.get("audioBytes") ?? 34_600_000);
/** The space the meetings' folders take, and what was set aside next to them. */
export const meetingsStorageBytes = Number(
  params.get("storageBytes") ?? 1_234_000_000,
);
export const setAsideBytes = Number(params.get("setAsideBytes") ?? 0);
export const meetingRecording = params.get("meeting") === "recording";
export const systemAudio = params.get("systemAudio") ?? "available";
export const meetingFailed = params.get("meetingFailed") === "1";
/** `running`: an audio import is already under way when the page opens.
 * `fails=<error>`: the import is refused with that error. Otherwise it runs
 * at 42% until it is cancelled. */
export const importMode = params.get("import");
export const IMPORT_RUNNING = {
  file_name: "Quarterly review.m4a",
  fraction: 0.42,
};
/** Why the failed meeting failed; `parts_failed` by default. */
export const meetingError = (params.get("meetingError") ??
  "parts_failed") as ProcessingError;
/**
 * `offline=1`: Codex could not be reached, and the queue waits for the
 * connection: the failed meeting, and a ready one whose summary is pending.
 */
export const offline = params.get("offline") === "1";
export const analysis = params.get("analysis") ?? "available";
/** `0`: a Mac that cannot tell a call started (Intel, or before macOS 14). */
export const callDetection = params.get("callDetection") !== "0";
/** `failed` (rate limited), `server` or `pending`; ready by default. */
export const meetingSummary = params.get("summary");
/** The stage saved for the processing meeting; `transcribing` by default. */
export const processingStage = (params.get("stage") ??
  "transcribing") as MeetingStage;
/**
 * `held=1` keeps the queued meetings behind a Codex rate-limit hold: the
 * processing one, and a ready one whose summary is pending.
 */
export const heldUntilMs =
  params.get("held") === "1" ? Date.now() + 2 * 60 * 60 * 1000 : null;
/** Enough older meetings to fill more than one page of the list. */
export const manyMeetings = params.get("manyMeetings") === "1";
export const speakerAnalysis = (params.get("speakers") ??
  "separated") as SpeakerAnalysis;
/** `local` or `cloud`; cloud by default, as for every existing user. */
export const engine = (params.get("engine") ?? "cloud") as Engine;
/** `engineRefused=1`: the backend refuses every engine change. */
export const engineRefused = params.get("engineRefused") === "1";
/** `cleanup=off` turns AI cleanup off; `style` is the stored dictation style. */
export const cleanupEnabled = params.get("cleanup") !== "off";
export const dictationStyle = (params.get("style") ??
  "normal") as DictationStyle;
/** `styleRefused=1`: the backend refuses every style change. */
export const styleRefused = params.get("styleRefused") === "1";
/** `card=1` shows the one-time local transcription card. */
export const cardShown = params.get("card") === "1";
/** `history=fallback`: entries transcribed by the engine not chosen. */
export const historyFallback = params.get("history") === "fallback";
/** `localFallback=1`: part of a meeting was transcribed on this Mac. */
export const localFallbackMs =
  params.get("localFallback") === "1" ? 12 * 60_000 : 0;
/** The updater finds a new version, which the footer offers. */
export const updateAvailable = params.get("update") === "1";
/** `missedUpdate=1`: a check finds a new version the footer never heard of. */
export const missedUpdate = params.get("missedUpdate") === "1";
/** `pending=check_updates,…`: what happened while the window was closed. */
export const pendingMainActions = new Set(
  params.get("pending")?.split(",").filter(Boolean) ?? [],
);
/** `lastSection=history`: the section a closed window showed last. */
export const lastSection = params.get("lastSection");
export const openMeeting = params.get("openMeeting");
