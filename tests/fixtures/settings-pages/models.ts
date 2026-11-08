import type {
  AnalysisSupport,
  DownloadFailure,
  LocalModelStatus,
} from "../../../src/bindings";
import { analysis, params } from "./params";

export const UPDATE = {
  body: "",
  currentVersion: "1.3.7",
  date: null,
  rawJson: {},
  rid: 1,
  version: "1.9.0",
};
/** The download as the manifest gives it, on Intel (`unsupported`) or Apple silicon. */
export const MODEL_BYTES =
  analysis === "unsupported" ? 739_508_576 : 505_044_208;
export const failed = (
  reason: DownloadFailure,
  kept_bytes = 0,
): LocalModelStatus => ({
  kept_bytes,
  size_bytes: MODEL_BYTES,
  state: { reason, state: "failed" },
});
export const MODELS: Record<string, LocalModelStatus> = {
  downloading: {
    kept_bytes: 0,
    size_bytes: MODEL_BYTES,
    state: {
      done_bytes: 212_000_000,
      state: "downloading",
      total_bytes: MODEL_BYTES,
    },
  },
  missing: {
    kept_bytes: 0,
    size_bytes: MODEL_BYTES,
    state: { state: "missing" },
  },
  /** A cancelled download whose partial files stay to be resumed. */
  cancelled: {
    kept_bytes: 212_000_000,
    size_bytes: MODEL_BYTES,
    state: { state: "missing" },
  },
  preparing: {
    kept_bytes: 0,
    size_bytes: MODEL_BYTES,
    state: { state: "preparing" },
  },
  ready: { kept_bytes: 0, size_bytes: MODEL_BYTES, state: { state: "ready" } },
  /** Compiled again for a new macOS version, at launch. */
  optimizing: {
    kept_bytes: 0,
    size_bytes: MODEL_BYTES,
    state: { state: "optimizing" },
  },
  failed: failed({ kind: "no_internet" }),
  serverDown: failed({ kind: "server_unavailable" }),
  damaged: failed({ kind: "damaged" }),
  disk: failed({ kind: "disk" }),
  noSpace: failed({
    free_bytes: 120_000_000,
    kind: "not_enough_space",
    needed_bytes: 555_548_628,
  }),
  /** Downloaded and checked, then would not load: every file stays. */
  prepareFailed: failed({ kind: "prepare_failed" }, MODEL_BYTES),
};
export const MODEL_COMMANDS: Record<string, LocalModelStatus> = {
  download_local_model: MODELS.downloading,
  cancel_local_model_download: MODELS.missing,
  delete_local_model: MODELS.missing,
};
// Intel Macs (`unsupported`) download the model for dictation, without
// speaker detection; macOS 14 has neither.
export const modelsStatus: LocalModelStatus =
  analysis !== "os_too_old"
    ? MODELS[params.get("models") ?? "ready"]
    : {
        kept_bytes: 0,
        size_bytes: MODEL_BYTES,
        state: {
          state: "unsupported",
          support: analysis as AnalysisSupport,
        },
      };
