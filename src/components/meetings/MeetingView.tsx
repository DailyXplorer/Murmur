import React, { useCallback, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { ArrowLeftIcon } from "@phosphor-icons/react/dist/csr/ArrowLeft";
import {
  commands,
  events,
  type MeetingDetail,
  type LocalModelStatus,
  type MeetingProgress,
  type MeetingStage,
  type SystemAudioPermission,
} from "@/bindings";
import { downloadPercent, useLocalModel } from "../../hooks/useLocalModel";
import { Button } from "../ui/Button";
import { panelId, tabId, Tabs } from "../ui/Tabs";
import { AudioRow } from "./AudioRow";
import { RenameSpeakersDialog } from "./RenameSpeakersDialog";
import { SummaryTab } from "./SummaryTab";
import { TranscriptTab } from "./TranscriptTab";

type Tab = "summary" | "transcript";
const TABS_ID = "meeting";

/** The stage saved with the meeting, before any live progress arrives. */
const SAVED_STAGE_LABELS: Record<MeetingStage, string> = {
  transcribing: "meetings.processing.transcribingStarted",
  analyzing: "meetings.processing.analyzing",
  aligning: "meetings.processing.aligning",
};

const formatChipDate = (milliseconds: number, locale: string) =>
  new Intl.DateTimeFormat(locale, {
    month: "short",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
  }).format(new Date(milliseconds));

export const formatMeetingDate = (milliseconds: number, locale: string) =>
  new Intl.DateTimeFormat(locale, {
    year: "numeric",
    month: "long",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
  }).format(new Date(milliseconds));

/** A heading that edits in place: Enter or leaving saves, Escape reverts. */
const TitleField: React.FC<{
  title: string | null;
  placeholder: string;
  onCommit: (title: string) => void;
}> = ({ title, placeholder, onCommit }) => {
  const { t } = useTranslation();
  const [draft, setDraft] = useState(title ?? "");
  // The title when editing began, `null` while the field is not focused: a
  // title saved meanwhile (the summary's) never replaces what is typed.
  const editedFrom = useRef<string | null>(null);
  useEffect(() => {
    if (editedFrom.current === null) setDraft(title ?? "");
  }, [title]);

  return (
    <input
      className="w-full truncate rounded-md border border-transparent bg-transparent px-1 py-0.5 text-[28px] font-semibold leading-tight text-text placeholder:text-text/50 hover:border-mid-gray/20 focus:border-logo-primary focus:outline-none"
      value={draft}
      placeholder={placeholder}
      maxLength={120}
      aria-label={t("meetings.view.titleLabel")}
      onChange={(event) => setDraft(event.target.value)}
      onFocus={() => {
        editedFrom.current = title ?? "";
      }}
      onBlur={() => {
        const from = editedFrom.current ?? "";
        editedFrom.current = null;
        if (draft.trim() !== from) onCommit(draft);
        else setDraft(title ?? "");
      }}
      onKeyDown={(event) => {
        if (event.key === "Enter") event.currentTarget.blur();
        if (event.key === "Escape") {
          event.preventDefault();
          editedFrom.current = title ?? "";
          setDraft(title ?? "");
          // Blur after the revert has rendered, so nothing is committed.
          const field = event.currentTarget;
          requestAnimationFrame(() => field.blur());
        }
      }}
    />
  );
};

const ProcessingBanner: React.FC<{
  meeting: MeetingDetail;
  progress: MeetingProgress | null;
  /** Absent when the meeting cannot be processed again. */
  onRetry?: () => void;
  /** Sends a held meeting past the hold, its summary too. */
  onProcessNow: () => void;
}> = ({ meeting, progress, onRetry, onProcessNow }) => {
  const { t, i18n } = useTranslation();
  // A ready meeting is held too while it waits for its summary.
  if (meeting.held_until_ms !== null) {
    const time = new Intl.DateTimeFormat(i18n.language, {
      hour: "2-digit",
      minute: "2-digit",
    }).format(new Date(meeting.held_until_ms));
    return (
      <div
        role="status"
        className="flex items-center gap-3 rounded-xl border border-mid-gray/20 px-4 py-3"
      >
        <p className="flex-1 text-sm">
          {t("meetings.processing.held", { time })}
        </p>
        <Button onClick={onProcessNow}>
          {t("meetings.processing.processNow")}
        </Button>
      </div>
    );
  }
  if (meeting.status === "processing") {
    const fraction =
      progress && progress.chunks_total > 0
        ? progress.chunks_done / progress.chunks_total
        : 0;
    // Until the next live update, the stage saved with the meeting: the
    // analysis reports progress only when it starts.
    const label = !progress
      ? t(
          meeting.stage
            ? SAVED_STAGE_LABELS[meeting.stage]
            : "meetings.processing.waiting",
        )
      : progress.rate_limited
        ? t("meetings.processing.rateLimited")
        : progress.stage === "aligning"
          ? t("meetings.processing.aligning")
          : progress.stage === "analyzing"
            ? t("meetings.processing.analyzing")
            : t("meetings.processing.transcribing", {
                done: progress.chunks_done,
                total: progress.chunks_total,
              });
    return (
      <div
        role="status"
        className="space-y-2 rounded-xl border border-mid-gray/20 px-4 py-3"
      >
        <p className="text-sm">{label}</p>
        <div
          role="progressbar"
          aria-label={label}
          aria-valuemin={0}
          aria-valuemax={100}
          aria-valuenow={Math.round(fraction * 100)}
          className="h-1.5 overflow-hidden rounded-full bg-mid-gray/20"
        >
          <div
            className="h-full rounded-full bg-logo-primary transition-[width] duration-300"
            style={{ width: `${Math.round(fraction * 100)}%` }}
          />
        </div>
      </div>
    );
  }
  if (meeting.status === "interrupted" || meeting.status === "failed") {
    const message =
      meeting.status === "interrupted"
        ? t("meetings.processing.interrupted")
        : t(`meetings.processing.errors.${meeting.error ?? "other"}`);
    return (
      <div
        role="alert"
        className="flex items-center gap-3 rounded-xl border border-mid-gray/20 px-4 py-3"
      >
        <p className="flex-1 text-sm">{message}</p>
        {onRetry && (
          <Button onClick={onRetry}>
            {meeting.status === "interrupted"
              ? t("meetings.processing.process")
              : t("meetings.processing.retry")}
          </Button>
        )}
      </div>
    );
  }
  return null;
};

/** Says why speakers were not told apart, and offers to do it now. */
const SpeakerNotice: React.FC<{
  meeting: MeetingDetail;
  models: LocalModelStatus | null;
  onSeparate: () => void;
  starting: boolean;
}> = ({ meeting, models, onSeparate, starting }) => {
  const { t } = useTranslation();
  const analysis = meeting.speaker_analysis;
  if (
    meeting.status === "processing" ||
    (analysis !== "no_models" && analysis !== "failed")
  ) {
    return null;
  }
  const download = async () => {
    const result = await commands.downloadLocalModel();
    if (result.status === "error") toast.error(t("meetings.models.error"));
  };
  const state = models?.state;
  const action = (() => {
    switch (state?.state) {
      case "ready":
        return (
          <Button onClick={onSeparate} disabled={starting}>
            {t("meetings.speakers.separate")}
          </Button>
        );
      case "missing":
      case "failed":
        return (
          <Button onClick={() => void download()}>
            {t("meetings.models.download")}
          </Button>
        );
      case "downloading":
        return (
          <span className="text-sm text-text/60 tabular-nums">
            {t("meetings.models.downloading", {
              percent: downloadPercent(models),
            })}
          </span>
        );
      case "preparing":
        return (
          <span className="text-sm text-text/60">
            {t("meetings.models.preparing")}
          </span>
        );
      default:
        return null;
    }
  })();
  return (
    <div
      role="status"
      className="flex items-center gap-3 rounded-xl border border-mid-gray/20 px-4 py-3"
    >
      <p className="flex-1 text-sm">{t(`meetings.speakers.${analysis}`)}</p>
      {action}
    </div>
  );
};

/**
 * One quiet line when system audio held only silence. Nothing else playing is
 * the usual cause, so only a missing permission gets a link to fix it.
 */
const SystemAudioNotice: React.FC = () => {
  const { t } = useTranslation();
  const [permission, setPermission] = useState<SystemAudioPermission | null>(
    null,
  );
  useEffect(() => {
    void commands.getSystemAudioPermission().then(setPermission);
  }, []);
  if (permission === null) return null;
  return (
    <p role="status" className="px-1 text-xs text-text/60">
      {permission === "granted" ? (
        t("meetings.permission.silentAllowed")
      ) : (
        <>
          {t("meetings.permission.silent")}{" "}
          <button
            type="button"
            className="cursor-pointer rounded-sm font-medium text-text underline decoration-logo-primary decoration-2 underline-offset-2 [text-decoration-skip-ink:none] hover:text-logo-primary focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-logo-primary"
            onClick={() => void commands.openSystemAudioPrivacySettings()}
          >
            {t("meetings.permission.open")}
          </button>
        </>
      )}
    </p>
  );
};

interface MeetingViewProps {
  id: number;
  onBack: () => void;
}

export const MeetingView: React.FC<MeetingViewProps> = ({ id, onBack }) => {
  const { t, i18n } = useTranslation();
  const [meeting, setMeeting] = useState<MeetingDetail | null>(null);
  const [failed, setFailed] = useState(false);
  const [progress, setProgress] = useState<MeetingProgress | null>(null);
  // Until the user picks a tab: the summary once the meeting is ready.
  const [chosenTab, setTab] = useState<Tab | null>(null);
  const [renaming, setRenaming] = useState<{ focus: string | null } | null>(
    null,
  );
  const models = useLocalModel();

  const load = useCallback(async () => {
    try {
      const result = await commands.getMeeting(id);
      if (result.status === "error") throw new Error(result.error);
      setMeeting(result.data);
      if (result.data.status !== "processing") setProgress(null);
    } catch (error) {
      console.error("Failed to load the meeting:", error);
      setFailed(true);
    }
  }, [id]);

  useEffect(() => {
    void load();
    const updated = events.meetingsUpdated.listen((event) => {
      if (event.payload.id === id) void load();
    });
    const progressed = events.meetingProgress.listen((event) => {
      if (event.payload.id === id) setProgress(event.payload);
    });
    return () => {
      void updated.then((stop) => stop());
      void progressed.then((stop) => stop());
    };
  }, [id, load]);

  // The hold lifts on its own: nothing announces it to the waiting meetings.
  const heldUntil = meeting?.held_until_ms ?? null;
  useEffect(() => {
    if (heldUntil === null) return;
    const timer = setTimeout(
      () => void load(),
      Math.max(0, heldUntil - Date.now()) + 1000,
    );
    return () => clearTimeout(timer);
  }, [heldUntil, load]);

  const retry = async () => {
    const result = await commands.retryMeeting(id);
    if (result.status === "error") {
      toast.error(t("meetings.processing.retryError"));
      return;
    }
    setProgress(null);
    void load();
  };

  // Separate speakers and the summary's Retry stay disabled until the
  // meeting reloads in its new state: a second click would find the action
  // already started and report it as failed.
  const [starting, setStarting] = useState(false);
  const startOnce = async (action: () => Promise<void>) => {
    if (starting) return;
    setStarting(true);
    try {
      await action();
    } finally {
      setStarting(false);
    }
  };

  const separate = async () => {
    const result = await commands.reanalyzeMeeting(id);
    if (result.status === "error") {
      toast.error(t("meetings.speakers.error"));
      return;
    }
    setProgress(null);
    await load();
  };

  const rename = async (title: string) => {
    const result = await commands.renameMeeting(id, title);
    if (result.status === "error") toast.error(t("meetings.view.renameError"));
    void load();
  };

  const back = (
    <Button size="chip" onClick={onBack}>
      {/* eslint-disable-next-line murmur/no-icon-in-text-button -- the one text button that keeps its icon */}
      <ArrowLeftIcon size={15} aria-hidden="true" />
      <span>{t("meetings.view.back")}</span>
    </Button>
  );

  if (!meeting) {
    return (
      <div className="mx-auto w-full max-w-3xl space-y-4">
        {back}
        <p className="px-1 text-sm text-mid-gray">
          {failed ? t("meetings.view.loadError") : t("common.loading")}
        </p>
      </div>
    );
  }

  const defaultTitle = t("meetings.list.untitled", {
    date: formatMeetingDate(meeting.started_at_ms, i18n.language),
  });
  const tab =
    chosenTab ?? (meeting.status === "ready" ? "summary" : "transcript");
  const hasAudio = meeting.audio !== "deleted";

  const retrySummary = async () => {
    const result = await commands.retryMeetingSummary(id);
    if (result.status === "error")
      toast.error(t("meetings.summary.retryError"));
    await load();
  };

  return (
    <article
      aria-label={meeting.title ?? defaultTitle}
      className="mx-auto w-full max-w-3xl space-y-4 pb-1"
    >
      {back}
      <div className="space-y-2">
        <TitleField
          title={meeting.title}
          placeholder={defaultTitle}
          onCommit={(title) => void rename(title)}
        />
        <span className="ml-1 inline-flex rounded-md border border-mid-gray/20 px-2 py-0.5 text-xs text-mid-gray tabular-nums">
          {formatChipDate(meeting.started_at_ms, i18n.language)}
        </span>
      </div>
      <ProcessingBanner
        meeting={meeting}
        progress={progress}
        onRetry={hasAudio ? () => void retry() : undefined}
        onProcessNow={() => void retry()}
      />
      {hasAudio && (
        <SpeakerNotice
          meeting={meeting}
          models={models}
          onSeparate={() => void startOnce(separate)}
          starting={starting}
        />
      )}
      {meeting.system_audio_silent && <SystemAudioNotice />}
      <Tabs<Tab>
        idPrefix={TABS_ID}
        label={t("meetings.view.tabs.label")}
        selected={tab}
        onSelect={setTab}
        tabs={[
          { id: "summary", label: t("meetings.view.tabs.summary") },
          { id: "transcript", label: t("meetings.view.tabs.transcript") },
        ]}
      />
      <div
        role="tabpanel"
        id={panelId(TABS_ID, tab)}
        aria-labelledby={tabId(TABS_ID, tab)}
      >
        {tab === "summary" ? (
          <SummaryTab
            meeting={meeting}
            onRetry={() => void startOnce(retrySummary)}
            retrying={starting}
          />
        ) : (
          <TranscriptTab
            meeting={meeting}
            onRenameSpeaker={(focus) => setRenaming({ focus })}
            onRetry={hasAudio ? () => void retry() : undefined}
          />
        )}
      </div>
      <AudioRow meeting={meeting} onChanged={() => void load()} />
      <RenameSpeakersDialog
        meetingId={meeting.id}
        speakers={meeting.speakers}
        focusKey={renaming?.focus ?? null}
        open={renaming !== null}
        onClose={() => setRenaming(null)}
        onSaved={() => void load()}
      />
    </article>
  );
};
