import React, { useMemo } from "react";
import { useTranslation } from "react-i18next";
import { ClockIcon } from "@phosphor-icons/react/dist/csr/Clock";
import { CopyIcon } from "@phosphor-icons/react/dist/csr/Copy";
import { DownloadSimpleIcon } from "@phosphor-icons/react/dist/csr/DownloadSimple";
import { UserGearIcon } from "@phosphor-icons/react/dist/csr/UserGear";
import { WarningIcon } from "@phosphor-icons/react/dist/csr/Warning";
import { type MeetingDetail } from "@/bindings";
import { formatDuration } from "@/lib/meetings/format";
import { speakerColor } from "@/lib/meetings/speakerColors";
import { Button } from "../ui/Button";
import { IconButton } from "../ui/IconButton";
import { TabHeaderBar, useTextSearch } from "./TabHeaderBar";
import { useMeetingMarkdown } from "./useMeetingMarkdown";

/** The speaker key of a part that could not be transcribed. */
export const PLACEHOLDER_SPEAKER = "_system";

interface TranscriptTabProps {
  meeting: MeetingDetail;
  /** Opens the rename dialog on this speaker, or on the first when `null`. */
  onRenameSpeaker: (key: string | null) => void;
  /** Absent when the meeting cannot be processed again. */
  onRetry?: () => void;
}

export const TranscriptTab: React.FC<TranscriptTabProps> = ({
  meeting,
  onRenameSpeaker,
  onRetry,
}) => {
  const { t } = useTranslation();
  const { copy, exportMarkdown } = useMeetingMarkdown(meeting.id, "transcript");

  const speakers = useMemo(
    () => new Map(meeting.speakers.map((speaker) => [speaker.key, speaker])),
    [meeting.speakers],
  );
  const speaking = useMemo(
    () =>
      new Set(
        meeting.turns
          .map(({ speaker_key }) => speaker_key)
          .filter((key) => key !== PLACEHOLDER_SPEAKER),
      ).size,
    [meeting.turns],
  );
  const texts = useMemo(
    () =>
      meeting.turns.map((turn) =>
        turn.speaker_key === PLACEHOLDER_SPEAKER ? "" : turn.text,
      ),
    [meeting.turns],
  );
  const search = useTextSearch(texts);

  const hasText = meeting.turns.some(
    (turn) => turn.speaker_key !== PLACEHOLDER_SPEAKER,
  );

  return (
    <div className="space-y-3">
      <TabHeaderBar
        search={search}
        icon={<ClockIcon size={15} />}
        label={t("meetings.transcript.header", {
          count: speaking,
          duration: formatDuration(meeting.duration_ms),
        })}
        searchLabel={t("meetings.transcript.search")}
        hasText={hasText}
      >
        <IconButton
          label={t("meetings.transcript.copy")}
          disabled={!hasText}
          onClick={() => void copy()}
        >
          <CopyIcon size={15} aria-hidden="true" />
        </IconButton>
        <IconButton
          label={t("meetings.transcript.export")}
          disabled={!hasText}
          onClick={() => void exportMarkdown()}
        >
          <DownloadSimpleIcon size={15} aria-hidden="true" />
        </IconButton>
        <IconButton
          label={t("meetings.transcript.renameSpeakers")}
          disabled={meeting.speakers.length === 0}
          onClick={() => onRenameSpeaker(null)}
        >
          <UserGearIcon size={15} aria-hidden="true" />
        </IconButton>
      </TabHeaderBar>

      {meeting.turns.length === 0 ? (
        <p className="px-1 py-6 text-center text-sm text-mid-gray">
          {meeting.status === "ready" || meeting.status === "failed"
            ? t("meetings.transcript.empty")
            : t("meetings.transcript.pending")}
        </p>
      ) : (
        <ol
          className="space-y-1.5"
          aria-label={t("meetings.view.tabs.transcript")}
        >
          {meeting.turns.map((turn, index) => {
            if (turn.speaker_key === PLACEHOLDER_SPEAKER) {
              return (
                <li
                  key={index}
                  className="my-3 flex items-center gap-3 rounded-xl border border-dashed border-mid-gray/30 px-3 py-2 text-sm text-mid-gray"
                >
                  <WarningIcon
                    size={15}
                    className="shrink-0"
                    aria-hidden="true"
                  />
                  <span className="flex-1">
                    {t("meetings.transcript.missingPart", {
                      start: formatDuration(turn.start_ms),
                      end: formatDuration(turn.end_ms),
                    })}
                  </span>
                  {meeting.status === "failed" && onRetry && (
                    <Button size="chip" onClick={onRetry}>
                      {t("meetings.processing.retry")}
                    </Button>
                  )}
                </li>
              );
            }
            const speaker = speakers.get(turn.speaker_key);
            const name = speaker?.name ?? turn.speaker_key;
            const previous = meeting.turns[index - 1];
            const showName =
              !previous || previous.speaker_key !== turn.speaker_key;
            return (
              <li key={index}>
                {showName && (
                  <button
                    type="button"
                    className="mt-3 cursor-pointer rounded-sm text-sm font-medium hover:underline focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent-text"
                    style={{ color: speakerColor(speaker?.color_index ?? 0) }}
                    title={t("meetings.transcript.renameSpeaker", { name })}
                    onClick={() => onRenameSpeaker(turn.speaker_key)}
                  >
                    {name}
                  </button>
                )}
                <p className="mt-1 w-fit max-w-[85%] select-text whitespace-pre-wrap rounded-xl bg-mid-gray/10 px-3 py-2 text-sm leading-relaxed [contain-intrinsic-size:auto_2.5rem] [content-visibility:auto]">
                  {search.highlight(turn.text, index)}
                </p>
              </li>
            );
          })}
        </ol>
      )}
    </div>
  );
};
