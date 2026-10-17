/** Lowercase, without accents: "Été" and "ete" compare equal. */
const fold = (text: string) =>
  text.normalize("NFD").replace(/\p{M}/gu, "").toLowerCase();

/**
 * Where `query` occurs in `text`, ignoring case and accents, as
 * `[start, end)` offsets into `text`.
 */
export const findMatches = (
  text: string,
  query: string,
): Array<[number, number]> => {
  const needle = fold(query.trim());
  if (!needle) return [];
  // For each folded character, where its source character starts and ends.
  const starts: number[] = [];
  const ends: number[] = [];
  let folded = "";
  for (let index = 0; index < text.length;) {
    const character = String.fromCodePoint(text.codePointAt(index) ?? 0);
    const end = index + character.length;
    for (const piece of fold(character)) {
      folded += piece;
      for (let unit = 0; unit < piece.length; unit++) {
        starts.push(index);
        ends.push(end);
      }
    }
    index = end;
  }
  const matches: Array<[number, number]> = [];
  for (
    let found = folded.indexOf(needle);
    found !== -1;
    found = folded.indexOf(needle, found + needle.length)
  ) {
    matches.push([starts[found], ends[found + needle.length - 1]]);
  }
  return matches;
};
