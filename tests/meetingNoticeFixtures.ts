import type { MeetingNoticeLabels } from "../src/overlay/RecordingOverlay";

export const MEETING_LAYOUTS = ["inline", "stacked"] as const;

const ready = {
  tone: "ready",
  headline: "Q4 planning with Priya",
  detail: "Transcript ready",
  open: "Open",
  copy: "Copy",
  copied: "Copied",
  close: "Close",
} satisfies MeetingNoticeLabels;

export const MEETING_NOTICES = {
  ready,
  failed: {
    ...ready,
    tone: "failed",
    headline: "Couldn't transcribe the meeting",
    detail: null,
    copy: null,
  },
  batch: {
    ...ready,
    headline: "2 meetings ready",
    detail: null,
    copy: null,
  },
  mixed: {
    ...ready,
    headline: "2 meetings ready",
    detail: "1 couldn't be transcribed",
    copy: null,
  },
  long: {
    ...ready,
    headline:
      "Weekly sync with the design, research and platform teams about the Q1 roadmap",
  },
  french: {
    ...ready,
    headline: "Planification du T4 avec Priya",
    detail: "Transcription prête",
    open: "Ouvrir",
    copy: "Copier",
    copied: "Copié",
    close: "Fermer",
  },
} satisfies Record<string, MeetingNoticeLabels>;
