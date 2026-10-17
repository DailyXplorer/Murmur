/**
 * The summary as Murmur asks Codex to write it: a context paragraph, bold
 * section headings and bullets two levels deep. People appear as `[[key]]`
 * markers and show under their current name, so a rename changes the
 * summary at once.
 */

/** A run of text, bold or not. */
export interface Span {
  text: string;
  bold: boolean;
}

export type SummaryBlock =
  | { kind: "paragraph"; spans: Span[] }
  | { kind: "heading"; spans: Span[] }
  | { kind: "bullet"; level: 0 | 1; spans: Span[] };

const MARKER = /\[\[([^\][\s]+)\]\]/g;
const WORDS_PER_MINUTE = 220;

/** `[[s2]]` → the speaker's name, or the bare key when nobody has it. */
export const resolveMarkers = (
  text: string,
  names: ReadonlyMap<string, string>,
): string => text.replace(MARKER, (_, key: string) => names.get(key) ?? key);

/** Splits `**bold**` runs from plain text. */
const spans = (text: string): Span[] =>
  text
    .split(/(\*\*[^*]+\*\*)/)
    .filter((piece) => piece !== "")
    .map((piece) =>
      piece.startsWith("**") && piece.endsWith("**") && piece.length > 4
        ? { text: piece.slice(2, -2), bold: true }
        : { text: piece, bold: false },
    );

export const parseSummary = (
  markdown: string,
  names: ReadonlyMap<string, string>,
): SummaryBlock[] =>
  resolveMarkers(markdown, names)
    .split("\n")
    .filter((line) => line.trim() !== "")
    .map((line): SummaryBlock => {
      const bullet = /^(\s*)[-*+•]\s+(.*)$/.exec(line);
      if (bullet && !line.trimStart().startsWith("**")) {
        return {
          kind: "bullet",
          level: bullet[1].length >= 2 ? 1 : 0,
          spans: spans(bullet[2].trim()),
        };
      }
      const trimmed = line.trim();
      const heading =
        /^#{1,6}\s+(.+)$/.exec(trimmed) ?? /^\*\*([^*]+)\*\*:?$/.exec(trimmed);
      if (heading) {
        return {
          kind: "heading",
          spans: [{ text: heading[1].replace(/\*\*/g, ""), bold: true }],
        };
      }
      return { kind: "paragraph", spans: spans(trimmed) };
    });

/** Minutes to read the summary, at least one. */
export const readMinutes = (blocks: SummaryBlock[]): number => {
  const words = blocks
    .flatMap((block) => block.spans)
    .map((span) => span.text)
    .join(" ")
    .split(/\s+/)
    .filter(Boolean).length;
  return Math.max(1, Math.ceil(words / WORDS_PER_MINUTE));
};
