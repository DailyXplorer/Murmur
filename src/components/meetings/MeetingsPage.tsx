import React, { useCallback, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { RecordIcon } from "@phosphor-icons/react/dist/csr/Record";
import { StopIcon } from "@phosphor-icons/react/dist/csr/Stop";
import { TrashIcon } from "@phosphor-icons/react/dist/csr/Trash";
import {
  commands,
  events,
  type MeetingCaptureMode,
  type MeetingListItem,
  type MeetingRuntime,
  type MeetingStatus,
  type SystemAudioSupport,
} from "@/bindings";
import { formatDuration } from "@/lib/meetings/format";
import { formatMeetingDate, MeetingView } from "./MeetingView";
import { useSettings } from "../../hooks/useSettings";
import { Button } from "../ui/Button";
import { Dialog } from "../ui/Dialog";
import { Dropdown } from "../ui/Dropdown";
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

/** The current time, refreshed every second while `active`. */
const useNow = (active: boolean) => {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    if (!active) return;
    setNow(Date.now());
    const timer = window.setInterval(() => setNow(Date.now()), 1000);
    return () => window.clearInterval(timer);
  }, [active]);
  return now;
};

export const MeetingsPage: React.FC = () => {
  const [selected, setSelected] = useState<number | null>(null);
  return selected === null ? (
    <MeetingsList onOpen={setSelected} />
  ) : (
    <MeetingView id={selected} onBack={() => setSelected(null)} />
  );
};

const MeetingsList: React.FC<{ onOpen: (id: number) => void }> = ({
  onOpen,
}) => {
  const { t, i18n } = useTranslation();
  const { getSetting, updateSetting, isUpdating } = useSettings();
  const [runtime, setRuntime] = useState<MeetingRuntime>({ state: "idle" });
  const [systemAudio, setSystemAudio] =
    useState<SystemAudioSupport>("available");
  const [meetings, setMeetings] = useState<MeetingListItem[]>([]);
  const [hasMore, setHasMore] = useState(false);
  const [loading, setLoading] = useState(true);
  const [busy, setBusy] = useState(false);
  const [confirmDiscard, setConfirmDiscard] = useState(false);
  const [pendingDelete, setPendingDelete] = useState<MeetingListItem | null>(
    null,
  );
  const requestRef = useRef(0);
  const recording = runtime.state === "recording";
  const now = useNow(recording);

  const loadMeetings = useCallback(
    async (cursor: number | null) => {
      const request = ++requestRef.current;
      try {
        const result = await commands.getMeetings(cursor, PAGE_SIZE);
        if (request !== requestRef.current) return;
        if (result.status === "error") throw new Error(result.error);
        setMeetings((current) =>
          cursor === null
            ? result.data.meetings
            : [...current, ...result.data.meetings],
        );
        setHasMore(result.data.has_more);
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
      if (!disposed) setSystemAudio(capabilities.system_audio);
    });
    void loadMeetings(null);
    const unlisten = events.meetingStateChanged.listen((event) => {
      setRuntime(event.payload);
      void loadMeetings(null);
    });
    const updated = events.meetingsUpdated.listen(() => {
      void loadMeetings(null);
    });
    return () => {
      disposed = true;
      void unlisten.then((stop) => stop());
      void updated.then((stop) => stop());
    };
  }, [loadMeetings]);

  const run = async (
    action: () => Promise<{ status: "ok" } | { status: "error" }>,
    errorKey: string,
  ) => {
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
    setMeetings((current) => current.filter(({ id }) => id !== meeting.id));
    try {
      const result = await commands.deleteMeeting(meeting.id);
      if (result.status === "error") throw new Error(result.error);
    } catch (error) {
      console.error("Failed to delete meeting:", error);
      toast.error(t("meetings.deleteError"));
      void loadMeetings(null);
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
      <p className="px-1 text-sm text-text/60">{t("meetings.notice")}</p>

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
        >
          <div className="flex justify-end gap-2">
            {recording ? (
              <>
                <Button
                  variant="ghost"
                  disabled={busy}
                  onClick={() => setConfirmDiscard(true)}
                >
                  {t("meetings.discard")}
                </Button>
                <Button
                  variant="primary"
                  disabled={busy}
                  onClick={() =>
                    void run(commands.stopMeeting, "meetings.stopError")
                  }
                >
                  <StopIcon size={15} weight="fill" aria-hidden="true" />
                  <span>{t("meetings.stop")}</span>
                </Button>
              </>
            ) : (
              <Button
                variant="primary"
                disabled={busy}
                onClick={() =>
                  void run(commands.startMeeting, "meetings.startError")
                }
              >
                <RecordIcon size={15} weight="fill" aria-hidden="true" />
                <span>{t("meetings.start")}</span>
              </Button>
            )}
          </div>
        </SettingContainer>
        <ShortcutInput shortcutId="meeting_toggle" grouped={true} />
        <SettingContainer
          title={t("meetings.captureMode.title")}
          description={t("meetings.captureMode.description")}
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
        {systemAudioAvailable ? (
          <SettingContainer
            title={t("meetings.permission.title")}
            description={t("meetings.permission.description")}
            grouped={true}
          >
            <Button
              variant="secondary"
              onClick={() => void commands.openSystemAudioPrivacySettings()}
            >
              {t("meetings.permission.open")}
            </Button>
          </SettingContainer>
        ) : (
          <p className="px-4 pb-3 text-sm text-text/60">
            {t("meetings.systemAudioUnavailable")}
          </p>
        )}
      </SettingsGroup>

      <SettingsGroup title={t("meetings.list.title")}>
        {loading ? (
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
                    disabled={live}
                    onClick={() => onOpen(meeting.id)}
                    className="min-w-0 flex-1 cursor-pointer rounded-md text-left focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-logo-primary disabled:cursor-default"
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
                  <Button
                    variant="ghost"
                    size="icon"
                    disabled={live}
                    aria-label={t("meetings.delete")}
                    title={t("meetings.delete")}
                    onClick={() => setPendingDelete(meeting)}
                  >
                    <TrashIcon size={15} aria-hidden="true" />
                  </Button>
                </li>
              );
            })}
          </ul>
        )}
        {hasMore && !loading && (
          <div className="flex justify-center px-4 py-2">
            <Button
              variant="secondary"
              onClick={() =>
                void loadMeetings(meetings[meetings.length - 1]?.id ?? null)
              }
            >
              {t("meetings.list.loadMore")}
            </Button>
          </div>
        )}
      </SettingsGroup>

      <Dialog
        open={confirmDiscard}
        onOpenChange={setConfirmDiscard}
        title={t("meetings.discardDialog.title")}
        closeLabel={t("common.close")}
        footer={
          <>
            <Button
              variant="secondary"
              onClick={() => setConfirmDiscard(false)}
            >
              {t("meetings.discardDialog.keep")}
            </Button>
            <Button
              variant="danger"
              onClick={() => {
                setConfirmDiscard(false);
                void run(commands.discardMeeting, "meetings.stopError");
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
            <Button variant="secondary" onClick={() => setPendingDelete(null)}>
              {t("meetings.cancel")}
            </Button>
            <Button
              variant="danger"
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
