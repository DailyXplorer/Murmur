/** The section every release note ends with: where a Mac reader gets the
 * Windows build. Written as a plain line or as a heading; a checkout on
 * Windows may end its lines with a carriage return. */
const WINDOWS_SECTION = /^(?:#+[ \t]*)?Windows \(beta\)[ \t\r]*$/m;

const hasContent = (markdown: string): boolean =>
  markdown
    .split("\n")
    .some((line) => line.trim() !== "" && !line.trimStart().startsWith("#"));

/**
 * The note as the Windows app shows it: without its closing "Windows (beta)"
 * section, which tells where to download the installer that this PC just
 * ran. A note that says nothing else is shown whole.
 */
export const withoutWindowsSection = (markdown: string): string => {
  const section = WINDOWS_SECTION.exec(markdown);
  if (!section) return markdown;
  const rest = markdown.slice(0, section.index).trimEnd();
  return hasContent(rest) ? `${rest}\n` : markdown;
};
