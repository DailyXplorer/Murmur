import type { MeetingNoticeLabels } from "../src/overlay/RecordingOverlay";
import en from "../src/i18n/locales/en/translation.json" with { type: "json" };
import fr from "../src/i18n/locales/fr/translation.json" with { type: "json" };
import de from "../src/i18n/locales/de/translation.json" with { type: "json" };
import es from "../src/i18n/locales/es/translation.json" with { type: "json" };
import it from "../src/i18n/locales/it/translation.json" with { type: "json" };
import pt from "../src/i18n/locales/pt/translation.json" with { type: "json" };

export const MEETING_LOCALES = { en, fr, de, es, it, pt };

const actions = (strings: typeof en.tray) => ({
  open: strings.overlayOpen,
  copy: strings.overlayCopy,
  copied: strings.overlayCopied,
});

const ready = {
  tone: "ready",
  headline: "Q4 planning with Priya",
  detail: en.tray.overlayTranscriptReady,
  ...actions(en.tray),
  default_name: null,
  meeting_id: 7,
} satisfies MeetingNoticeLabels;

export const MEETING_NOTICES = {
  ready,
  failed: {
    ...ready,
    tone: "failed",
    headline: en.tray.overlayMeetingFailed,
    detail: ready.headline,
    copy: null,
  },
  batch: {
    ...ready,
    meeting_id: null,
    headline: en.tray.overlayMeetingsReady_other.replace("{{count}}", "2"),
    detail: null,
    copy: null,
  },
  mixed: {
    ...ready,
    meeting_id: null,
    headline: en.tray.overlayMeetingsReady_other.replace("{{count}}", "2"),
    detail: en.tray.overlayMeetingsUntranscribed_one.replace("{{count}}", "1"),
    copy: null,
  },
  failedBatch: {
    ...ready,
    meeting_id: null,
    tone: "failed",
    headline: en.tray.overlayMeetingsFailed_other.replace("{{count}}", "2"),
    detail: null,
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
    detail: fr.tray.overlayTranscriptReady,
    ...actions(fr.tray),
  },
} satisfies Record<string, MeetingNoticeLabels>;

export const untitledNotice = (
  locale: keyof typeof MEETING_LOCALES,
  startedAtMs: number,
  failed: boolean,
): MeetingNoticeLabels => {
  const strings = MEETING_LOCALES[locale];
  return {
    ...ready,
    ...actions(strings.tray),
    tone: failed ? "failed" : "ready",
    headline: failed
      ? strings.tray.overlayMeetingFailed
      : strings.meetings.list.untitled,
    detail: failed
      ? strings.meetings.list.untitled
      : strings.tray.overlayTranscriptReady,
    copy: failed ? null : strings.tray.overlayCopy,
    default_name: { started_at_ms: startedAtMs, locale },
  };
};
