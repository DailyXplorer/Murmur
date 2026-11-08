import type {
  LiveState,
  LiveTranscript,
  MeetingDetail,
  MeetingHit,
  MeetingListItem,
  MeetingLiveUpdated,
  MeetingRuntime,
  Snippet,
} from "../../../src/bindings";
import { events } from "../../../src/bindings";
import { findMatches } from "../../../src/lib/meetings/search";
import { HOUR, NOW } from "./history";
import {
  audioBytes,
  heldUntilMs,
  localFallbackMs,
  manyMeetings,
  meetingError,
  meetingFailed,
  meetingRecording,
  meetingSummary,
  offline,
  params,
  processingStage,
  speakerAnalysis,
  systemAudioSilent,
} from "./params";

const MINUTE_MS = 60_000;
const MEETINGS: MeetingListItem[] = [
  {
    duration_ms: meetingRecording ? 0 : 47 * MINUTE_MS + 12_000,
    id: 4,
    started_at_ms: NOW * 1000 - 50 * MINUTE_MS,
    status: meetingRecording ? "recording" : "processing",
    title: null,
    tracks: "both",
  },
  {
    duration_ms: 72 * MINUTE_MS + 5_000,
    id: 3,
    started_at_ms: NOW * 1000 - 26 * HOUR * 1000,
    status: meetingFailed ? "failed" : "ready",
    title: "Point hebdo produit",
    tracks: "both",
  },
  {
    duration_ms: 9 * MINUTE_MS + 41_000,
    id: 2,
    started_at_ms: NOW * 1000 - 74 * HOUR * 1000,
    status: "interrupted",
    title: null,
    tracks: "both",
  },
  ...(manyMeetings
    ? Array.from({ length: 60 }, (_, index) => ({
        duration_ms: 30 * MINUTE_MS,
        id: -index - 1,
        started_at_ms: NOW * 1000 - (100 + index) * HOUR * 1000,
        status: "ready" as const,
        title: `Older meeting ${index + 1}`,
        tracks: "both" as const,
      }))
    : []),
];
export const MEETING_RUNTIME: MeetingRuntime = meetingRecording
  ? {
      id: 4,
      started_at_ms: Date.now() - 12 * MINUTE_MS - 34_000,
      state: "recording",
      stops_at_ms: Date.now() + 3 * HOUR * 1000,
      tracks: "both",
    }
  : { state: "idle" };

const SECOND_MS = 1000;
const turn = (speaker_key: string, start_s: number, text: string) => ({
  end_ms: (start_s + 4) * SECOND_MS,
  low_confidence: true,
  speaker_key,
  start_ms: start_s * SECOND_MS,
  text,
});
/** A fictional summary, as Codex writes it: people as markers. */
const SUMMARY = `Weekly product check-in between [[me]] and [[s1]].

**Home page**
- The new home page is ready for Thursday's review
  - Two screens left, nothing blocking

**Payment integration**
- Already in testing; production planned for early November

**Action items**
- [[s1]] — tell the support team
- [[s1]] — send the invite for the sales demo — next week`;

const summaryOf = (item: MeetingListItem) => {
  if (
    item.id !== 3 ||
    item.status !== "ready" ||
    meetingSummary === "pending"
  ) {
    return {
      summary: null,
      summary_error: null,
      summary_status: "pending" as const,
    };
  }
  return meetingSummary === "failed" || meetingSummary === "server"
    ? {
        summary: null,
        summary_error:
          meetingSummary === "server"
            ? ("server" as const)
            : ("rate_limited" as const),
        summary_status: "failed" as const,
      }
    : {
        summary: SUMMARY,
        summary_error: null,
        summary_status: "ready" as const,
      };
};

const queuedForSummary = (item: MeetingListItem) =>
  item.status === "ready" && summaryOf(item).summary_status === "pending";

const detailOf = (item: MeetingListItem): MeetingDetail => ({
  ...item,
  ...summaryOf(item),
  audio: item.id === 3 ? "compressed" : "tracks",
  audio_bytes: item.id === 3 ? audioBytes : 0,
  system_audio_silent: item.id === 3 && systemAudioSilent,
  error: item.status === "failed" ? meetingError : null,
  held_until_ms:
    item.status === "processing" || queuedForSummary(item) ? heldUntilMs : null,
  waiting_for_connection:
    offline && (item.status === "failed" || queuedForSummary(item)),
  local_fallback_ms: item.id === 3 ? localFallbackMs : 0,
  speakers:
    item.id === 3
      ? [
          { color_index: 0, default_name: "Me", key: "me", name: "Me" },
          {
            color_index: 1,
            default_name: "Speaker 2",
            key: "s1",
            name: "Speaker 2",
          },
        ]
      : [],
  speaker_analysis: item.id === 3 ? speakerAnalysis : null,
  stage: item.status === "processing" ? processingStage : null,
  turns:
    item.id === 3
      ? [
          turn("s1", 4, "Bonjour à tous, on commence par le point produit ?"),
          turn(
            "me",
            9,
            "Oui. La nouvelle page d'accueil est prête pour la revue de jeudi.",
          ),
          turn("me", 15, "Il reste deux écrans à finaliser, rien de bloquant."),
          turn(
            "s1",
            31,
            "Parfait. Et l'intégration avec la plateforme de paiement ?",
          ),
          turn(
            "me",
            38,
            "Elle est déjà en test. On vise une mise en production début novembre.",
          ),
          ...(meetingFailed
            ? [{ ...turn("_system", 600, ""), end_ms: 1_200_000 }]
            : []),
          turn("s1", 1_262, "Super, je préviens l'équipe support."),
          turn(
            "me",
            1_270,
            "Let's also schedule a quick demo for the sales team next week.",
          ),
          turn("s1", 1_278, "Sounds good, I'll send the invite."),
        ]
      : [],
});
export const DETAILS = new Map(
  MEETINGS.map((item) => [item.id, detailOf(item)]),
);

const liveParam = params.get("live") ?? "starting";
const LIVE_STATES = ["starting", "live", "catching_up", "stopped"];
const LIVE_DRAFTS = new Map<number, LiveTranscript>(
  meetingRecording
    ? [
        [
          4,
          {
            entries: [],
            state: (LIVE_STATES.includes(liveParam)
              ? { state: liveParam }
              : { reason: liveParam, state: "unavailable" }) as LiveState,
          },
        ],
      ]
    : [],
);
export const emitLive = (update: MeetingLiveUpdated) => {
  const draft = LIVE_DRAFTS.get(update.id) ?? {
    entries: [],
    state: update.state,
  };
  LIVE_DRAFTS.set(update.id, {
    entries: [...draft.entries, ...update.appended],
    state: update.state,
  });
  return events.meetingLiveUpdated.emit(update);
};
const snippetOf = (text: string, query: string): Snippet => ({
  text,
  ranges: findMatches(text, query),
});

/** Commands whose answer depends on their arguments or changes the data. */
let meetingLoadsHeld: Promise<void> | null = null;
let releaseLoads = () => {};
const heldMeetingDetails = new Map<
  number,
  { waiting: Promise<void>; release: () => void }
>();

export const holdMeetingLoads = () => {
  meetingLoadsHeld = new Promise((resolve) => {
    releaseLoads = () => {
      meetingLoadsHeld = null;
      resolve();
    };
  });
};
export const releaseMeetingLoads = () => releaseLoads();
export const holdMeetingDetail = (id: number) => {
  let release = () => {};
  const waiting = new Promise<void>((resolve) => {
    release = resolve;
  });
  heldMeetingDetails.set(id, { waiting, release });
};
export const releaseMeetingDetail = (id: number) => {
  const held = heldMeetingDetails.get(id);
  heldMeetingDetails.delete(id);
  held?.release();
};

export const handleMeetingCommand = (
  command: string,
  args: Record<string, unknown>,
): { handled: boolean; value?: unknown } => {
  const meeting = DETAILS.get(args.id as number);
  switch (command) {
    case "get_meetings": {
      // Pages of at most 50, newest first, after the `cursor` id.
      const limit = Math.min((args.limit as number | null) ?? 50, 50);
      const after = MEETINGS.slice(
        MEETINGS.findIndex(({ id }) => id === args.cursor) + 1,
      );
      const page = {
        has_more: after.length > limit,
        meetings: after.slice(0, limit),
      };
      return {
        handled: true,
        value: meetingLoadsHeld ? meetingLoadsHeld.then(() => page) : page,
      };
    }
    case "delete_meeting":
      MEETINGS.splice(
        MEETINGS.findIndex(({ id }) => id === args.id),
        1,
      );
      return { handled: true, value: null };
    case "get_meeting": {
      // A copy, as the real IPC returns: React sees each load as new data.
      const detail = structuredClone(meeting);
      const held = meeting ? heldMeetingDetails.get(meeting.id) : undefined;
      return {
        handled: true,
        value: held ? held.waiting.then(() => detail) : detail,
      };
    }
    case "rename_meeting":
      if (meeting) {
        const title = String(args.title).trim();
        meeting.title = title === "" ? null : title;
      }
      return { handled: true, value: null };
    case "rename_speaker":
      meeting?.speakers.forEach((speaker) => {
        if (speaker.key === args.key) speaker.name = String(args.name);
      });
      return { handled: true, value: null };
    case "retry_meeting_summary":
      // Codex answers at once.
      if (meeting) {
        Object.assign(meeting, {
          summary: SUMMARY,
          summary_error: null,
          summary_status: "ready",
        });
      }
      return { handled: true, value: null };
    case "delete_meeting_audio":
      if (meeting) Object.assign(meeting, { audio: "deleted", audio_bytes: 0 });
      return { handled: true, value: null };
    case "get_live_transcript":
      return {
        handled: true,
        value: structuredClone(LIVE_DRAFTS.get(args.id as number) ?? null),
      };
    case "search_meetings": {
      // Titles and turns holding the query, newest meeting first.
      const query = String(args.query);
      const hits: MeetingHit[] = MEETINGS.flatMap((item) => {
        const title = item.title ? snippetOf(item.title, query) : null;
        const turns = (DETAILS.get(item.id)?.turns ?? [])
          .map((turn, seq) => ({ seq, snippet: snippetOf(turn.text, query) }))
          .filter(({ snippet }) => snippet.ranges.length > 0);
        if (!title?.ranges.length && turns.length === 0) return [];
        return [
          {
            duration_ms: item.duration_ms,
            first_turn_seq: turns[0]?.seq ?? null,
            id: item.id,
            snippets: turns.slice(0, 2).map(({ snippet }) => snippet),
            started_at_ms: item.started_at_ms,
            status: item.status,
            title,
          },
        ];
      });
      return { handled: true, value: hits };
    }
    default:
      return { handled: false };
  }
};
