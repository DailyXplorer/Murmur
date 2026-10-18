import React, { useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { CaretDownIcon } from "@phosphor-icons/react/dist/csr/CaretDown";
import { CaretUpIcon } from "@phosphor-icons/react/dist/csr/CaretUp";
import { MagnifyingGlassIcon } from "@phosphor-icons/react/dist/csr/MagnifyingGlass";
import { XIcon } from "@phosphor-icons/react/dist/csr/X";
import { findMatches } from "@/lib/meetings/search";
import { IconButton, IconToolbar } from "../ui/IconButton";
import { Input } from "../ui/Input";

interface Match {
  item: number;
  start: number;
  end: number;
}

/**
 * Search across the texts of a tab, ignoring case and accents. `highlight`
 * renders one text with its matches marked; the current one scrolls into
 * view.
 */
export const useTextSearch = (texts: string[]) => {
  const [searching, setSearching] = useState(false);
  const [query, setQuery] = useState("");
  const [chosen, setChosen] = useState(0);
  const currentMark = useRef<HTMLElement | null>(null);

  const matches = useMemo<Match[]>(
    () =>
      searching
        ? texts.flatMap((text, item) =>
            findMatches(text, query).map(([start, end]) => ({
              item,
              start,
              end,
            })),
          )
        : [],
    [texts, query, searching],
  );
  const byItem = useMemo(() => {
    const map = new Map<number, Array<Match & { index: number }>>();
    matches.forEach((match, index) => {
      const list = map.get(match.item) ?? [];
      list.push({ ...match, index });
      map.set(match.item, list);
    });
    return map;
  }, [matches]);

  // The text can change under the search and leave fewer matches.
  const current = Math.min(chosen, Math.max(matches.length - 1, 0));

  useEffect(() => setChosen(0), [query]);
  useEffect(() => {
    currentMark.current?.scrollIntoView({ block: "center" });
  }, [current, matches]);

  const step = (by: number) => {
    if (matches.length === 0) return;
    setChosen((current + by + matches.length) % matches.length);
  };

  const close = () => {
    setSearching(false);
    setQuery("");
  };

  const highlight = (text: string, item: number): React.ReactNode => {
    const own = byItem.get(item);
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

  return {
    searching,
    open: () => setSearching(true),
    close,
    query,
    setQuery,
    count: matches.length,
    current,
    step,
    highlight,
  };
};

export type TextSearch = ReturnType<typeof useTextSearch>;

interface TabHeaderBarProps {
  search: TextSearch;
  icon: React.ReactNode;
  label: string;
  searchLabel: string;
  hasText: boolean;
  /** Copy, export and the other actions of the tab. */
  children: React.ReactNode;
}

/** The bar above a tab: what it holds, or the search field, then actions. */
export const TabHeaderBar: React.FC<TabHeaderBarProps> = ({
  search,
  icon,
  label,
  searchLabel,
  hasText,
  children,
}) => {
  const { t } = useTranslation();
  return (
    <div className="flex min-h-11 items-center gap-2 rounded-xl bg-mid-gray/10 py-1 pl-3 pr-2">
      {search.searching ? (
        <>
          <Input
            autoFocus
            className="h-8 min-w-0 flex-1"
            value={search.query}
            placeholder={t("meetings.transcript.searchPlaceholder")}
            aria-label={searchLabel}
            onChange={(event) => search.setQuery(event.target.value)}
            onKeyDown={(event) => {
              if (event.key === "Enter") {
                event.preventDefault();
                search.step(event.shiftKey ? -1 : 1);
              } else if (event.key === "Escape") {
                event.preventDefault();
                search.close();
              }
            }}
          />
          <span
            className="shrink-0 text-xs tabular-nums text-mid-gray"
            aria-live="polite"
          >
            {search.query.trim() === ""
              ? null
              : search.count > 0
                ? t("meetings.transcript.matches", {
                    current: search.current + 1,
                    total: search.count,
                  })
                : t("meetings.transcript.noMatches")}
          </span>
        </>
      ) : (
        <>
          <span className="flex shrink-0 text-mid-gray" aria-hidden="true">
            {icon}
          </span>
          <span className="min-w-0 flex-1 truncate text-xs font-medium uppercase tracking-wide text-mid-gray tabular-nums">
            {label}
          </span>
        </>
      )}
      <IconToolbar>
        {search.searching ? (
          <>
            <IconButton
              label={t("meetings.transcript.previousMatch")}
              disabled={search.count === 0}
              onClick={() => search.step(-1)}
            >
              <CaretUpIcon size={15} aria-hidden="true" />
            </IconButton>
            <IconButton
              label={t("meetings.transcript.nextMatch")}
              disabled={search.count === 0}
              onClick={() => search.step(1)}
            >
              <CaretDownIcon size={15} aria-hidden="true" />
            </IconButton>
            <IconButton
              label={t("meetings.transcript.closeSearch")}
              onClick={search.close}
            >
              <XIcon size={15} aria-hidden="true" />
            </IconButton>
          </>
        ) : (
          <IconButton
            label={searchLabel}
            disabled={!hasText}
            onClick={search.open}
          >
            <MagnifyingGlassIcon size={15} aria-hidden="true" />
          </IconButton>
        )}
        {children}
      </IconToolbar>
    </div>
  );
};
