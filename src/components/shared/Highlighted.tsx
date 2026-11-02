import React from "react";

interface HighlightedProps {
  text: string;
  /** Sorted, non-overlapping `[start, end)` offsets into `text`. */
  ranges: [number, number][];
}

/** `text` with each of `ranges` marked. */
export const Highlighted: React.FC<HighlightedProps> = ({ text, ranges }) => {
  const pieces: React.ReactNode[] = [];
  let at = 0;
  for (const [start, end] of ranges) {
    // The ranges cross IPC: one that does not fit the text marks nothing.
    if (start < at || end <= start || end > text.length) continue;
    pieces.push(
      text.slice(at, start),
      <mark key={start} className="rounded-sm bg-logo-primary/25 text-text">
        {text.slice(start, end)}
      </mark>,
    );
    at = end;
  }
  pieces.push(text.slice(at));
  return <>{pieces}</>;
};
