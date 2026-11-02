import React, { useCallback, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { TrashIcon } from "@phosphor-icons/react/dist/csr/Trash";
import {
  commands,
  events,
  type MeetingCaptureMode,
  type MeetingHit,
  type MeetingListItem,
  type MeetingRuntime,
  type MeetingStatus,
  type AnalysisSupport,
  type SystemAudioSupport,
} from "@/bindings";
import { formatDuration } from "@/lib/meetings/format";
import { findMatches } from "@/lib/meetings/search";
import { LiveMeetingView } from "./LiveMeetingView";
import { formatMeetingDate, MeetingView } from "./MeetingView";
import { SpeakerModels } from "./SpeakerModels";
import type { InitialTextSearch } from "./TabHeaderBar";
import { useLocalModel } from "../../hooks/useLocalModel";
import { useNow } from "../../hooks/useNow";
import { type SearchState, useSearch } from "../../hooks/useSearch";
import { Highlighted, SearchField } from "../shared";
import { useSettings } from "../../hooks/useSettings";
import { Button } from "../ui/Button";
import { Dialog } from "../ui/Dialog";
import { Dropdown } from "../ui/Dropdown";
import { IconButton } from "../ui/IconButton";
import { SettingContainer } from "../ui/SettingContainer";
import { SettingsGroup } from "../ui/SettingsGroup";
import { SettingsPage } from "../ui/SettingsPage";
import { ShortcutInput } from "../settings/ShortcutInput";

const PAGE_SIZE = 50;

const STATUS_KEYS: Record<MeetingStatus, string> = {
  recording: "meetings.status.recording",
  interrupted: "meetings.status.interrupted",
  processing: "meetings.status.processing",
  ready: "meetings.status.ready",
  failed: "meetings.status.failed",
};

/**
 * What the transcript search opens with. It looks for the query as one
 * phrase, while the meeting search matches each word on its own: when the
 * phrase is not in the matching passage, the first word marked there is.
 */
const transcriptQuery = (hit: MeetingHit, query: string) => {
  // Quotes are search syntax; the transcript search reads them as text.
  const typed = query.replace(/"/g, "").trim();
  // Transcript passages come after the summary's.
  const passage =
    hit.first_turn_seq === null
      ? undefined
      : hit.snippets[hit.snippets.length - 1];
  const first = passage?.ranges[0];
  if (!passage || !first || findMatches(passage.text, typed).length > 0) {
    return typed;
  }
  return passage.text.slice(first[0], first[1]);
};

interface Opened {
  id: number;
  /** The meeting is recording: it opens in the live view. */
  live: boolean;
  initialSearch?: InitialTextSearch;
}

export const MeetingsPage: React.FC = () => {
  const [opened, setOpened] = useState<Opened | null>(null);
  const stopped = useCallback(
    () => setOpened((current) => current && { ...current, live: false }),
    [],
  );
  // Here rather than in the list, so Back returns to the same results.
  const [query, setQuery] = useState("");
  const search = useSearch(query, commands.searchMeetings);
  const { refresh } = search;
  useEffect(() => {
    const updated = events.meetingsUpdated.listen(refresh);
    return () => void updated.then((stop) => stop());
  }, [refresh]);
  if (opened === null) {
    return (
      <MeetingsList
        onOpen={(id, live) => setOpened({ id, live })}
        onOpenHit={(hit) =>
          setOpened({
            id: hit.id,
            live: false,
            initialSearch: {
              query: transcriptQuery(hit, query),
              item: hit.first_turn_seq,
            },
          })
        }
        query={query}
        onQueryChange={setQuery}
        search={search.state}
      />
    );
  }
  const back = () => {
    setOpened(null);
    refresh();
  };
  return opened.live ? (
    <LiveMeetingView id={opened.id} onBack={back} onStopped={stopped} />
  ) : (
    <MeetingView
      id={opened.id}
      initialSearch={opened.initialSearch}
      onBack={back}
    />
  );
};

interface MeetingsListProps {
  onOpen: (id: number, live: boolean) => void;
  onOpenHit: (hit: MeetingHit) => void;
  query: string;
  onQueryChange: (query: string) => void;
  search: SearchState<MeetingHit>;
}

const MeetingsList: React.FC<MeetingsListProps> = ({
  onOpen,
  onOpenHit,
  query,
  onQueryChange,
  search,
}) => {
  const { t, i18n } = useTranslation();
  const { getSetting, updateSetting, isUpdating } = useSettings();
  const [runtime, setRuntime] = useState<MeetingRuntime>({ state: "idle" });
  const [systemAudio, setSystemAudio] =
    useState<SystemAudioSupport>("available");
  const [analysis, setAnalysis] = useState<AnalysisSupport | null>(null);
  const [meetings, setMeetings] = useState<MeetingListItem[]>([]);
  const [hasMore, setHasMore] = useState(false);
  const [loading, setLoading] = useState(true);
  const [busy, setBusy] = useState(false);
  /** The meeting the Discard dialog was opened for. */
  const [discardId, setDiscardId] = useState<number | null>(null);
  const [pendingDelete, setPendingDelete] = useState<MeetingListItem | null>(
    null,
  );
  const requestRef = useRef(0);
  const models = useLocalModel();
  const recording = runtime.state === "recording";
  const now = useNow(recording);
  // The dialog closes by itself once its meeting stops some other way.
  const confirmDiscard =
    discardId !== null && recording && runtime.id === discardId;

  /** Rows the list shows: a page, plus one per Show More. */
  const shownRef = useRef(PAGE_SIZE);

  /** Loads the newest `count` meetings, page by page. Every load builds
   * the whole list, so the latest one can replace whatever came before. */
  const loadMeetings = useCallback(
    async (count: number) => {
      const request = ++requestRef.current;
      shownRef.current = count;
      try {
        const rows: MeetingListItem[] = [];
        let more = true;
        while (more && rows.length < count) {
          const result = await commands.getMeetings(
            rows[rows.length - 1]?.id ?? null,
            PAGE_SIZE,
          );
          if (request !== requestRef.current) return;
          if (result.status === "error") throw new Error(result.error);
          rows.push(...result.data.meetings);
          more = result.data.has_more && result.data.meetings.length > 0;
        }
        setMeetings(rows);
        setHasMore(more);
      } catch (error) {
        console.error("Failed to load meetings:", error);
        toast.error(t("meetings.loadError"));
      } finally {
        if (request === requestRef.current) setLoading(false);
      }
    },
    [t],
  );

  useEffect(() => {
    let disposed = false;
    void commands.getMeetingRuntime().then((current) => {
      if (!disposed) setRuntime(current);
    });
    void commands.getMeetingCapabilities().then((capabilities) => {
      if (disposed) return;
      setSystemAudio(capabilities.system_audio);
      setAnalysis(capabilities.analysis);
    });
    void loadMeetings(PAGE_SIZE);
    const unlisten = events.meetingStateChanged.listen((event) => {
      const next = event.payload;
      setRuntime(next);
      // A meeting that stopped takes its dialog with it: the next meeting
      // may get the same id.
      setDiscardId((id) =>
        next.state === "recording" && next.id === id ? id : null,
      );
      void loadMeetings(shownRef.current);
    });
    const updated = events.meetingsUpdated.listen(() => {
      void loadMeetings(shownRef.current);
    });
    return () => {
      disposed = true;
      void unlisten.then((stop) => stop());
      void updated.then((stop) => stop());
    };
  }, [loadMeetings]);

  // Start and Stop swap places as soon as the meeting changes state, so the
  // rest of a double click would undo the first one.
  const run = async (
    event: React.MouseEvent,
    action: () => Promise<{ status: "ok" } | { status: "error" }>,
    errorKey: string,
  ) => {
    if (event.detail > 1) return;
    setBusy(true);
    try {
      const result = await action();
      if (result.status === "error") toast.error(t(errorKey));
    } catch (error) {
      console.error("Meeting action failed:", error);
      toast.error(t(errorKey));
    } finally {
      setBusy(false);
    }
  };

  const deleteMeeting = async (meeting: MeetingListItem) => {
    setPendingDelete(null);
    // A load already on its way may still hold the meeting: drop its answer.
    // The backend announces the delete, which loads the list again.
    requestRef.current += 1;
    setMeetings((current) => current.filter(({ id }) => id !== meeting.id));
    try {
      const result = await commands.deleteMeeting(meeting.id);
      if (result.status === "error") throw new Error(result.error);
    } catch (error) {
      console.error("Failed to delete meeting:", error);
      toast.error(t("meetings.deleteError"));
      void loadMeetings(shownRef.current);
    }
  };

  const systemAudioAvailable = systemAudio === "available";
  const captureMode: MeetingCaptureMode = systemAudioAvailable
    ? (getSetting("meeting_capture_mode") ?? "both")
    : "mic_only";
  const captureOptions = (["both", "system_only", "mic_only"] as const).map(
    (value) => ({ value, label: t(`meetings.captureMode.options.${value}`) }),
  );

  return (
    <SettingsPage label={t("sidebar.meetings")}>
      <SettingsGroup title={t("meetings.recording.title")}>
        <SettingContainer
          title={
            runtime.state === "recording"
              ? t("meetings.recording.active", {
                  time: formatDuration(now - runtime.started_at_ms),
                })
              : t("meetings.recording.idle")
          }
          description={t("meetings.recording.description")}
          grouped={true}
          layout="stacked"
        >
          <div className="flex justify-end gap-2">
            {recording ? (
              <>
                <Button
                  disabled={busy}
                  onClick={(event) => {
                    if (event.detail <= 1) setDiscardId(runtime.id);
                  }}
                >
                  {t("meetings.discard")}
                </Button>
                <Button
                  disabled={busy}
                  onClick={(event) =>
                    void run(event, commands.stopMeeting, "meetings.stopError")
                  }
                >
                  {t("meetings.stop")}
                </Button>
              </>
            ) : (
              <Button
                disabled={busy}
                onClick={(event) =>
                  void run(event, commands.startMeeting, "meetings.startError")
                }
              >
                {t("meetings.start")}
              </Button>
            )}
          </div>
        </SettingContainer>
        <ShortcutInput shortcutId="meeting_toggle" grouped={true} />
        <ShortcutInput shortcutId="meeting_paste_new" grouped={true} />
        <SettingContainer
          title={t("meetings.captureMode.title")}
          description={
            systemAudioAvailable
              ? t("meetings.captureMode.description")
              : t("meetings.systemAudioUnavailable")
          }
          grouped={true}
        >
          <Dropdown
            options={captureOptions}
            selectedValue={captureMode}
            onSelect={(value) =>
              updateSetting("meeting_capture_mode", value as MeetingCaptureMode)
            }
            disabled={
              recording ||
              !systemAudioAvailable ||
              isUpdating("meeting_capture_mode")
            }
          />
        </SettingContainer>
        {systemAudioAvailable && (
          <SettingContainer
            title={t("meetings.permission.title")}
            description={t("meetings.permission.description")}
            grouped={true}
          >
            <Button
              onClick={() => void commands.openSystemAudioPrivacySettings()}
            >
              {t("meetings.permission.open")}
            </Button>
          </SettingContainer>
        )}
      </SettingsGroup>

      <SpeakerModels status={models} analysis={analysis} />

      <SettingsGroup title={t("meetings.list.title")}>
        {(meetings.length > 0 || query !== "") && (
          <div className="px-4 py-3">
            <SearchField
              value={query}
              onChange={onQueryChange}
              placeholder={t("search.placeholder.meetings")}
            />
          </div>
        )}
        {search.status !== "idle" ? (
          <MeetingHits
            search={search}
            query={query.trim()}
            liveId={recording ? runtime.id : null}
            onOpen={onOpenHit}
          />
        ) : loading ? (
          <div className="px-4 py-3 text-center text-text/60">
            {t("common.loading")}
          </div>
        ) : meetings.length === 0 ? (
          <div className="px-4 py-3 text-center text-text/60">
            {t("meetings.list.empty")}
          </div>
        ) : (
          <ul className="divide-y divide-mid-gray/20">
            {meetings.map((meeting) => {
              const date = formatMeetingDate(
                meeting.started_at_ms,
                i18n.language,
              );
              const live =
                runtime.state === "recording" && runtime.id === meeting.id;
              return (
                <li
                  key={meeting.id}
                  className="flex min-h-12 items-center gap-3 px-4 py-2"
                >
                  <button
                    type="button"
                    onClick={() => onOpen(meeting.id, live)}
                    className="min-w-0 flex-1 cursor-pointer rounded-md text-left focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent-text disabled:cursor-default"
                  >
                    <p className="truncate text-sm font-medium">
                      {meeting.title ?? t("meetings.list.untitled", { date })}
                    </p>
                    <p className="text-xs text-text/60 tabular-nums">
                      {t("meetings.list.details", {
                        duration: formatDuration(
                          live
                            ? now - meeting.started_at_ms
                            : meeting.duration_ms,
                        ),
                        status: t(STATUS_KEYS[meeting.status]),
                      })}
                    </p>
                  </button>
                  <IconButton
                    disabled={live}
                    label={t("meetings.delete")}
                    onClick={() => setPendingDelete(meeting)}
                  >
                    <TrashIcon size={15} aria-hidden="true" />
                  </IconButton>
                </li>
              );
            })}
          </ul>
        )}
        {hasMore && !loading && search.status === "idle" && (
          <div className="flex justify-center px-4 py-2">
            <Button
              onClick={() => void loadMeetings(meetings.length + PAGE_SIZE)}
            >
              {t("meetings.list.loadMore")}
            </Button>
          </div>
        )}
      </SettingsGroup>

      <Dialog
        open={confirmDiscard}
        onOpenChange={(open) => {
          if (!open) setDiscardId(null);
        }}
        title={t("meetings.discardDialog.title")}
        closeLabel={t("common.close")}
        footer={
          <>
            <Button onClick={() => setDiscardId(null)}>
              {t("meetings.discardDialog.keep")}
            </Button>
            <Button
              onClick={(event) => {
                if (discardId === null) return;
                setDiscardId(null);
                void run(
                  event,
                  () => commands.discardMeeting(discardId),
                  "meetings.stopError",
                );
              }}
            >
              {t("meetings.discard")}
            </Button>
          </>
        }
      >
        <p className="text-sm text-text/80">
          {t("meetings.discardDialog.description")}
        </p>
      </Dialog>

      <Dialog
        open={pendingDelete !== null}
        onOpenChange={(open) => {
          if (!open) setPendingDelete(null);
        }}
        title={t("meetings.deleteDialog.title")}
        closeLabel={t("common.close")}
        footer={
          <>
            <Button onClick={() => setPendingDelete(null)}>
              {t("meetings.cancel")}
            </Button>
            <Button
              onClick={() => pendingDelete && void deleteMeeting(pendingDelete)}
            >
              {t("common.delete")}
            </Button>
          </>
        }
      >
        <p className="text-sm text-text/80">
          {t("meetings.deleteDialog.description")}
        </p>
      </Dialog>
    </SettingsPage>
  );
};

/** The search's answer in place of the list: one row per meeting. */
const MeetingHits: React.FC<{
  search: Exclude<SearchState<MeetingHit>, { status: "idle" }>;
  query: string;
  /** The meeting being recorded, which cannot be opened yet. */
  liveId: number | null;
  onOpen: (hit: MeetingHit) => void;
}> = ({ search, query, liveId, onOpen }) => {
  const { t, i18n } = useTranslation();
  if (search.status === "error" || search.hits.length === 0) {
    return (
      <div className="px-4 py-3 text-center text-text/60">
        {search.status === "error"
          ? t("search.failed")
          : search.status === "loading"
            ? t("common.loading")
            : t("search.noResults", { query })}
      </div>
    );
  }
  return (
    <ul className="divide-y divide-mid-gray/20">
      {search.hits.map((hit) => {
        const date = formatMeetingDate(hit.started_at_ms, i18n.language);
        const details = t("meetings.list.details", {
          duration: formatDuration(hit.duration_ms),
          status: t(STATUS_KEYS[hit.status]),
        });
        return (
          <li key={hit.id} className="px-4 py-2">
            <button
              type="button"
              disabled={hit.id === liveId}
              onClick={() => onOpen(hit)}
              className="w-full min-w-0 cursor-pointer rounded-md text-left focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent-text disabled:cursor-default"
            >
              <p className="truncate text-sm font-medium">
                {hit.title ? (
                  <Highlighted {...hit.title} />
                ) : (
                  t("meetings.list.untitled", { date })
                )}
              </p>
              <p className="text-xs text-text/60 tabular-nums">
                {hit.title ? `${date} · ${details}` : details}
              </p>
              {hit.snippets.map((snippet, index) => (
                <p
                  key={index}
                  className="mt-1 line-clamp-2 text-xs text-text/60"
                >
                  <Highlighted {...snippet} />
                </p>
              ))}
            </button>
          </li>
        );
      })}
    </ul>
  );
};
