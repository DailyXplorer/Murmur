import React, { useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { CaretDownIcon } from "@phosphor-icons/react/dist/csr/CaretDown";
import { CaretUpIcon } from "@phosphor-icons/react/dist/csr/CaretUp";
import { ClockIcon } from "@phosphor-icons/react/dist/csr/Clock";
import { CopyIcon } from "@phosphor-icons/react/dist/csr/Copy";
import { DownloadSimpleIcon } from "@phosphor-icons/react/dist/csr/DownloadSimple";
import { MagnifyingGlassIcon } from "@phosphor-icons/react/dist/csr/MagnifyingGlass";
import { UserGearIcon } from "@phosphor-icons/react/dist/csr/UserGear";
import { WarningIcon } from "@phosphor-icons/react/dist/csr/Warning";
import { XIcon } from "@phosphor-icons/react/dist/csr/X";
import { commands, type MeetingDetail } from "@/bindings";
import { formatDuration } from "@/lib/meetings/format";
import { findMatches } from "@/lib/meetings/search";
import { speakerColor } from "@/lib/meetings/speakerColors";
import { Button } from "../ui/Button";
import { Input } from "../ui/Input";

/** The speaker key of a part that could not be transcribed. */
export const PLACEHOLDER_SPEAKER = "_system";

interface Match {
  turn: number;
  start: number;
  end: number;
}

interface TranscriptTabProps {
  meeting: MeetingDetail;
  /** Opens the rename dialog on this speaker, or on the first when `null`. */
  onRenameSpeaker: (key: string | null) => void;
  onRetry: () => void;
}

const IconButton: React.FC<{
  label: string;
  onClick: () => void;
  disabled?: boolean;
  children: React.ReactNode;
}> = ({ label, onClick, disabled, children }) => (
  <Button
    variant="ghost"
    size="icon"
    aria-label={label}
    title={label}
    disabled={disabled}
    onClick={onClick}
  >
    {children}
  </Button>
);

export const TranscriptTab: React.FC<TranscriptTabProps> = ({
  meeting,
  onRenameSpeaker,
  onRetry,
}) => {
  const { t } = useTranslation();
  const [searching, setSearching] = useState(false);
  const [query, setQuery] = useState("");
  const [current, setCurrent] = useState(0);
  const currentMark = useRef<HTMLElement | null>(null);

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
  const matches = useMemo<Match[]>(
    () =>
      searching
        ? meeting.turns.flatMap((turn, index) =>
            turn.speaker_key === PLACEHOLDER_SPEAKER
              ? []
              : findMatches(turn.text, query).map(([start, end]) => ({
                  turn: index,
                  start,
                  end,
                })),
          )
        : [],
    [meeting.turns, query, searching],
  );
  const matchesByTurn = useMemo(() => {
    const byTurn = new Map<number, Array<Match & { index: number }>>();
    matches.forEach((match, index) => {
      const list = byTurn.get(match.turn) ?? [];
      list.push({ ...match, index });
      byTurn.set(match.turn, list);
    });
    return byTurn;
  }, [matches]);

  useEffect(() => setCurrent(0), [query]);
  useEffect(() => {
    currentMark.current?.scrollIntoView({ block: "center" });
  }, [current, matches]);

  const step = (by: number) => {
    if (matches.length === 0) return;
    setCurrent((index) => (index + by + matches.length) % matches.length);
  };

  const closeSearch = () => {
    setSearching(false);
    setQuery("");
  };

  const copy = async () => {
    try {
      const result = await commands.getMeetingMarkdown(meeting.id);
      if (result.status === "error") throw new Error(result.error);
      await navigator.clipboard.writeText(result.data);
      toast.success(t("meetings.transcript.copied"));
    } catch (error) {
      console.error("Failed to copy the transcript:", error);
      toast.error(t("meetings.transcript.copyError"));
    }
  };

  const exportMarkdown = async () => {
    try {
      const result = await commands.exportMeetingMarkdown(meeting.id);
      if (result.status === "error") throw new Error(result.error);
      if (result.data) toast.success(t("meetings.transcript.exported"));
    } catch (error) {
      console.error("Failed to export the transcript:", error);
      toast.error(t("meetings.transcript.exportError"));
    }
  };

  const renderText = (text: string, turn: number) => {
    const own = matchesByTurn.get(turn);
    if (!own) return text;
    const pieces: React.ReactNode[] = [];
    let at = 0;
    for (const match of own) {
      pieces.push(text.slice(at, match.start));
      const isCurrent = match.index === current;
      pieces.push(
        <mark
          key={match.index}
          ref={(element) => {
            if (isCurrent) currentMark.current = element;
          }}
          data-current={isCurrent || undefined}
          className={`rounded-sm text-text ${
            isCurrent ? "bg-logo-primary/60" : "bg-logo-primary/25"
          }`}
        >
          {text.slice(match.start, match.end)}
        </mark>,
      );
      at = match.end;
    }
    pieces.push(text.slice(at));
    return pieces;
  };

  const hasText = meeting.turns.some(
    (turn) => turn.speaker_key !== PLACEHOLDER_SPEAKER,
  );

  return (
    <div className="space-y-3">
      <div className="flex min-h-11 items-center gap-2 rounded-xl bg-mid-gray/10 px-3 py-1">
        {searching ? (
          <>
            <Input
              autoFocus
              className="h-8 min-w-0 flex-1"
              value={query}
              placeholder={t("meetings.transcript.searchPlaceholder")}
              aria-label={t("meetings.transcript.search")}
              onChange={(event) => setQuery(event.target.value)}
              onKeyDown={(event) => {
                if (event.key === "Enter") {
                  event.preventDefault();
                  step(event.shiftKey ? -1 : 1);
                } else if (event.key === "Escape") {
                  event.preventDefault();
                  closeSearch();
                }
              }}
            />
            <span
              className="shrink-0 text-xs tabular-nums text-mid-gray"
              aria-live="polite"
            >
              {query.trim() === ""
                ? null
                : matches.length > 0
                  ? t("meetings.transcript.matches", {
                      current: current + 1,
                      total: matches.length,
                    })
                  : t("meetings.transcript.noMatches")}
            </span>
            <IconButton
              label={t("meetings.transcript.previousMatch")}
              disabled={matches.length === 0}
              onClick={() => step(-1)}
            >
              <CaretUpIcon size={15} aria-hidden="true" />
            </IconButton>
            <IconButton
              label={t("meetings.transcript.nextMatch")}
              disabled={matches.length === 0}
              onClick={() => step(1)}
            >
              <CaretDownIcon size={15} aria-hidden="true" />
            </IconButton>
            <IconButton
              label={t("meetings.transcript.closeSearch")}
              onClick={closeSearch}
            >
              <XIcon size={15} aria-hidden="true" />
            </IconButton>
          </>
        ) : (
          <>
            <ClockIcon
              size={15}
              className="shrink-0 text-mid-gray"
              aria-hidden="true"
            />
            <span className="min-w-0 flex-1 truncate text-xs font-medium uppercase tracking-wide text-mid-gray tabular-nums">
              {t("meetings.transcript.header", {
                count: speaking,
                duration: formatDuration(meeting.duration_ms),
              })}
            </span>
            <IconButton
              label={t("meetings.transcript.search")}
              disabled={!hasText}
              onClick={() => setSearching(true)}
            >
              <MagnifyingGlassIcon size={15} aria-hidden="true" />
            </IconButton>
          </>
        )}
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
      </div>

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
                  {meeting.status === "failed" && (
                    <Button variant="secondary" size="chip" onClick={onRetry}>
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
                    className="mt-3 cursor-pointer rounded-sm text-sm font-medium hover:underline focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-logo-primary"
                    style={{ color: speakerColor(speaker?.color_index ?? 0) }}
                    title={t("meetings.transcript.renameSpeaker", { name })}
                    onClick={() => onRenameSpeaker(turn.speaker_key)}
                  >
                    {name}
                  </button>
                )}
                <p className="mt-1 w-fit max-w-[85%] select-text whitespace-pre-wrap rounded-xl bg-mid-gray/10 px-3 py-2 text-sm leading-relaxed [contain-intrinsic-size:auto_2.5rem] [content-visibility:auto]">
                  {renderText(turn.text, index)}
                </p>
              </li>
            );
          })}
        </ol>
      )}
    </div>
  );
};
