import React from "react";
import { useTranslation } from "react-i18next";
import { type LiveEntry } from "@/bindings";
import { speakerColor } from "@/lib/meetings/speakerColors";

export const LIVE_LABEL_KEYS = {
  mic: "meetings.live.you",
  system: "meetings.live.others",
} as const;

export const LiveEntries: React.FC<{
  entries: LiveEntry[];
  muted?: boolean;
}> = ({ entries, muted = false }) => {
  const { t } = useTranslation();
  return (
    <ol
      className={`space-y-1.5 ${muted ? "text-mid-gray" : ""}`}
      aria-label={t("meetings.live.title")}
    >
      {entries.map((entry, index) => {
        const key = `${entry.source}:${entry.start_ms}:${entry.end_ms}:${entry.kind}:${entry.text}`;
        if (entry.kind === "gap") {
          return (
            <li key={key} className="px-1 py-1 text-sm italic text-mid-gray">
              {t("meetings.live.gap")}
            </li>
          );
        }
        const previous = entries[index - 1];
        const showLabel =
          !previous ||
          previous.kind === "gap" ||
          previous.source !== entry.source;
        return (
          <li key={key}>
            {showLabel && (
              <p
                className="mt-3 text-sm font-medium"
                style={
                  muted
                    ? undefined
                    : { color: speakerColor(entry.source === "mic" ? 0 : 1) }
                }
              >
                {t(LIVE_LABEL_KEYS[entry.source])}
              </p>
            )}
            <p className="mt-1 w-fit max-w-[85%] select-text whitespace-pre-wrap rounded-xl bg-mid-gray/10 px-3 py-2 text-sm leading-relaxed">
              {entry.text}
            </p>
          </li>
        );
      })}
    </ol>
  );
};
