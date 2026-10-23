import React, { useMemo } from "react";
import { useTranslation } from "react-i18next";
import { BookOpenTextIcon } from "@phosphor-icons/react/dist/csr/BookOpenText";
import { CopyIcon } from "@phosphor-icons/react/dist/csr/Copy";
import { DownloadSimpleIcon } from "@phosphor-icons/react/dist/csr/DownloadSimple";
import { type MeetingDetail } from "@/bindings";
import {
  parseSummary,
  readMinutes,
  type Span,
  type SummaryBlock,
} from "@/lib/meetings/summary";
import { Button } from "../ui/Button";
import { FailureLine } from "../ui/FailureLine";
import { IconButton } from "../ui/IconButton";
import { TabHeaderBar, useTextSearch } from "./TabHeaderBar";
import { useMeetingMarkdown } from "./useMeetingMarkdown";

interface SummaryTabProps {
  meeting: MeetingDetail;
  onRetry: () => void;
  retrying: boolean;
}

type Group =
  | { kind: "list"; items: Array<SummaryBlock & { kind: "bullet" }> }
  | SummaryBlock;

/** Consecutive bullets form one list. */
const group = (blocks: SummaryBlock[]): Group[] =>
  blocks.reduce<Group[]>((groups, block) => {
    const last = groups[groups.length - 1];
    if (block.kind === "bullet") {
      if (last?.kind === "list") last.items.push(block);
      else groups.push({ kind: "list", items: [block] });
    } else {
      groups.push(block);
    }
    return groups;
  }, []);

const SummaryState: React.FC<{
  meeting: MeetingDetail;
  onRetry: () => void;
  retrying: boolean;
}> = ({ meeting, onRetry, retrying }) => {
  const { t } = useTranslation();
  if (meeting.summary_status === "failed") {
    const error = meeting.summary_error ?? "other";
    return (
      <FailureLine
        className="px-1 py-2"
        message={t(`meetings.summary.errors.${error}`)}
        action={
          // Only a Ready meeting gets its summary written; a failed one
          // comes back with the meeting's own retry.
          meeting.status === "ready" &&
          error !== "nothing_to_summarize" && (
            <Button onClick={onRetry} disabled={retrying}>
              {t("meetings.processing.retry")}
            </Button>
          )
        }
      />
    );
  }
  if (meeting.status === "ready") {
    // The banner above says when a held summary resumes.
    if (meeting.held_until_ms !== null || meeting.waiting_for_connection)
      return null;
    return (
      <div role="status" className="space-y-3 px-1 py-2">
        <p className="text-sm text-mid-gray">{t("meetings.summary.writing")}</p>
        {[92, 78, 85, 60].map((width) => (
          <div
            key={width}
            className="h-3 animate-pulse rounded-full bg-mid-gray/15"
            style={{ width: `${width}%` }}
          />
        ))}
      </div>
    );
  }
  return (
    <p className="px-1 py-6 text-center text-sm text-mid-gray">
      {t("meetings.summary.pending")}
    </p>
  );
};

export const SummaryTab: React.FC<SummaryTabProps> = ({
  meeting,
  onRetry,
  retrying,
}) => {
  const { t } = useTranslation();
  const { copy, exportMarkdown } = useMeetingMarkdown(meeting.id, "summary");

  const blocks = useMemo(
    () =>
      meeting.summary_status === "ready" && meeting.summary
        ? parseSummary(
            meeting.summary,
            new Map(
              meeting.speakers.map((speaker) => [speaker.key, speaker.name]),
            ),
          )
        : [],
    [meeting.summary, meeting.summary_status, meeting.speakers],
  );
  // Every span gets a search index, in reading order.
  const indexed = useMemo(() => {
    let next = 0;
    return blocks.map((block) =>
      block.spans.map((span) => ({ span, item: next++ })),
    );
  }, [blocks]);
  const texts = useMemo(
    () => blocks.flatMap((block) => block.spans.map((span) => span.text)),
    [blocks],
  );
  const search = useTextSearch(texts);
  const hasText = blocks.length > 0;

  const renderSpans = (spans: Array<{ span: Span; item: number }>) =>
    spans.map(({ span, item }) =>
      span.bold ? (
        <strong key={item} className="font-semibold text-text">
          {search.highlight(span.text, item)}
        </strong>
      ) : (
        <React.Fragment key={item}>
          {search.highlight(span.text, item)}
        </React.Fragment>
      ),
    );

  let position = 0;
  const groups = group(blocks).map((entry, key) => {
    if (entry.kind === "list") {
      const items = entry.items.map((item) => ({
        item,
        spans: indexed[position++],
      }));
      return (
        <ul key={key} className="space-y-1 ps-5">
          {items.map(({ item, spans }, index) => (
            <li
              key={index}
              className={
                item.level === 1
                  ? "ms-5 list-[circle] marker:text-text/40"
                  : "list-disc marker:text-text/50"
              }
            >
              {renderSpans(spans)}
            </li>
          ))}
        </ul>
      );
    }
    const spans = indexed[position++];
    return entry.kind === "heading" ? (
      <h3 key={key} className="pt-3 text-sm font-semibold text-text">
        {renderSpans(spans)}
      </h3>
    ) : (
      <p key={key}>{renderSpans(spans)}</p>
    );
  });

  return (
    <div className="space-y-3">
      <TabHeaderBar
        search={search}
        icon={<BookOpenTextIcon size={15} />}
        label={
          hasText
            ? t("meetings.summary.header", { count: readMinutes(blocks) })
            : t("meetings.view.tabs.summary")
        }
        searchLabel={t("meetings.summary.search")}
        hasText={hasText}
      >
        <IconButton
          label={t("meetings.summary.copy")}
          disabled={!hasText}
          onClick={() => void copy()}
        >
          <CopyIcon size={15} aria-hidden="true" />
        </IconButton>
        <IconButton
          label={t("meetings.summary.export")}
          disabled={!hasText}
          onClick={() => void exportMarkdown()}
        >
          <DownloadSimpleIcon size={15} aria-hidden="true" />
        </IconButton>
      </TabHeaderBar>
      {hasText ? (
        <section
          aria-label={t("meetings.view.tabs.summary")}
          className="select-text space-y-2 px-1 text-sm leading-relaxed text-text/85"
        >
          {groups}
        </section>
      ) : (
        <SummaryState meeting={meeting} onRetry={onRetry} retrying={retrying} />
      )}
    </div>
  );
};
