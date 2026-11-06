import { readFileSync } from "node:fs";

/** A directory that differs between runs, and the fixed name it is printed as. */
export interface PathMap {
  from: string;
  to: string;
}

/**
 * Rewrites the text of `rustc -Zunpretty=expanded` so two runs on the same
 * source are byte-identical: absolute directories become fixed names. (On the
 * app crate this is the only run-to-run difference: one `config_parent` path
 * in the code `tauri::generate_context!` expands to.)
 */
export function normalizeExpanded(text: string, maps: PathMap[]): string {
  const longestFirst = [...maps].sort((a, b) => b.from.length - a.from.length);
  let out = text;
  for (const { from, to } of longestFirst) {
    out = out.split(from.replace(/\/+$/, "")).join(to);
  }
  return out;
}

/**
 * `#[derive(specta::Type)]` writes where the type is declared into the code it
 * expands to, twice: `impl_location("src/a/b.rs:12:8")` and the second argument
 * of `sid(..)`, `"::murmur::a::b:12:8"`. Any edit above a type moves its line,
 * and a moved file changes its path, with the program unchanged. The module
 * path of a `sid` stays.
 */
export function normalizeLocations(text: string): string {
  return text
    .replace(/"[A-Za-z0-9_/.-]+\.rs:\d+:\d+"/g, '"<location>"')
    .replace(/"(::[A-Za-z0-9_:]+):\d+:\d+"/g, '"$1:<position>"');
}

type Lexical =
  | { kind: "code" }
  | { kind: "string" }
  | { kind: "raw"; hashes: number }
  | { kind: "block"; depth: number };

const isIdentChar = (char: string | undefined) =>
  char !== undefined && /[A-Za-z0-9_]/.test(char);

/** Moves the lexical state across one line of Rust text. */
function scanLine(line: string, start: Lexical): Lexical {
  let state = start;
  let index = 0;
  while (index < line.length) {
    const char = line[index] as string;
    const next = line[index + 1];
    if (state.kind === "string") {
      if (char === "\\") index += 1;
      else if (char === '"') state = { kind: "code" };
    } else if (state.kind === "raw") {
      if (
        char === '"' &&
        line.slice(index + 1, index + 1 + state.hashes) ===
          "#".repeat(state.hashes)
      ) {
        index += state.hashes;
        state = { kind: "code" };
      }
    } else if (state.kind === "block") {
      if (char === "/" && next === "*") {
        state = { kind: "block", depth: state.depth + 1 };
        index += 1;
      } else if (char === "*" && next === "/") {
        state =
          state.depth === 1
            ? { kind: "code" }
            : { kind: "block", depth: state.depth - 1 };
        index += 1;
      }
    } else if (char === "/" && next === "/") {
      break;
    } else if (char === "/" && next === "*") {
      state = { kind: "block", depth: 1 };
      index += 1;
    } else if (char === '"') {
      state = { kind: "string" };
    } else if (
      char === "r" &&
      !isIdentChar(line[index - 1] === "b" ? line[index - 2] : line[index - 1])
    ) {
      const raw = /^r(#*)"/.exec(line.slice(index));
      if (raw) {
        state = { kind: "raw", hashes: (raw[1] as string).length };
        index += raw[0].length - 1;
      }
    } else if (char === "'") {
      // A character literal ('a', '\n', '"') ends within a few characters; a lifetime does not.
      const literal =
        /^'(?:\\(?:u\{[0-9a-fA-F_]+\}|x[0-9a-fA-F]{2}|.)|[^'\\])'/.exec(
          line.slice(index),
        );
      if (literal) index += literal[0].length - 1;
    }
    index += 1;
  }
  return state;
}

/**
 * Drops what changes without the program changing: blank lines and doc
 * comments (`rustc` prints them as `///`). A line that starts inside a string
 * literal or a block comment is never dropped, so a blank line or a `///` line
 * inside a multi-line string still shows up in a diff.
 */
export function stripNoise(text: string): string {
  const kept: string[] = [];
  let state: Lexical = { kind: "code" };
  for (const line of text.split("\n")) {
    const startsInCode = state.kind === "code";
    state = scanLine(line, state);
    const trimmed = line.trim();
    if (startsInCode && (trimmed === "" || /^\/\/[/!]/.test(trimmed))) continue;
    kept.push(line);
  }
  return kept.join("\n");
}

export interface HunkSite {
  file: "base" | "head";
  line: number;
  /** The items that enclose the first changed line, outermost first. */
  path: string[];
}

export interface DiffSummary {
  hunks: number;
  /** Hunks per top-level item (a module, most of the time). */
  items: Record<string, number>;
  sites: HunkSite[];
}

const ITEM =
  /^(?:pub(?:\([^)]*\))?\s+)?(?:unsafe\s+)?(?:async\s+)?(?:const\s+)?(mod|fn|struct|enum|union|trait|impl|static|const|type|macro_rules!)\b\s*(?:<[^>]*>\s*)?([A-Za-z0-9_:<>]*)/;

const indentOf = (line: string) => line.length - line.trimStart().length;

/**
 * The items that enclose a line of rustfmt-formatted text, outermost first:
 * walking up, each nearer line that is less indented and opens an item.
 */
export function itemPathAt(lines: string[], lineNumber: number): string[] {
  const path: string[] = [];
  const at = Math.min(lineNumber, lines.length) - 1;
  let limit = indentOf(lines[at] ?? "") + 1;
  for (let index = at; index >= 0 && limit > 0; index--) {
    const line = lines[index] ?? "";
    if (line.trim() === "" || indentOf(line) >= limit) continue;
    const match = ITEM.exec(line.trim());
    if (match) {
      path.unshift(`${match[1]} ${match[2]}`.trim());
      limit = indentOf(line);
    }
  }
  return path;
}

/**
 * Counts the hunks of a unified diff and places each one: the items that
 * enclose its first changed line, read in the head file (the base file when the
 * line is a removal).
 */
export function summarizeDiff(
  diff: string,
  baseText: string,
  headText: string,
): DiffSummary {
  const lines = { base: baseText.split("\n"), head: headText.split("\n") };
  const items: Record<string, number> = {};
  const sites: HunkSite[] = [];
  let hunks = 0;
  let base = 0;
  let head = 0;
  let placed = true;
  for (const line of diff.split("\n")) {
    const header = /^@@ -(\d+)(?:,\d+)? \+(\d+)(?:,\d+)? @@/.exec(line);
    if (header) {
      hunks += 1;
      base = Number(header[1]);
      head = Number(header[2]);
      placed = false;
      continue;
    }
    if (hunks === 0 || line.startsWith("\\")) continue;
    const mark = line[0];
    if (!placed && (mark === "+" || mark === "-")) {
      const file = mark === "+" ? "head" : "base";
      const number = mark === "+" ? head : base;
      const path = itemPathAt(lines[file], number);
      const key = path[0] ?? "(crate root)";
      items[key] = (items[key] ?? 0) + 1;
      sites.push({ file, line: number, path });
      placed = true;
    }
    if (mark === " ") {
      base += 1;
      head += 1;
    } else if (mark === "-") base += 1;
    else if (mark === "+") head += 1;
  }
  return { hunks, items, sites };
}

const MAX_SITES = 100;

export function formatSummary(summary: DiffSummary): string {
  const names = Object.entries(summary.items)
    .sort((a, b) => b[1] - a[1] || a[0].localeCompare(b[0]))
    .map(([name, count]) => `  ${count}  ${name}`);
  const head = `summary: ${summary.hunks} hunk${summary.hunks === 1 ? "" : "s"}`;
  if (summary.sites.length === 0) return [head, ...names].join("\n");
  const sites = summary.sites
    .slice(0, MAX_SITES)
    .map(
      (site) =>
        `  ${site.file}:${site.line}  ${site.path.join(" > ") || "(crate root)"}`,
    );
  const more = summary.sites.length - sites.length;
  return [
    head,
    ...names,
    "first changed line of each hunk, in the head file (base file for a removal):",
    ...sites,
    ...(more > 0 ? [`  ... ${more} more`] : []),
  ].join("\n");
}

function parseMaps(args: string[]): PathMap[] {
  const maps: PathMap[] = [];
  for (let index = 0; index < args.length; index += 2) {
    if (args[index] !== "--map" || args[index + 1] === undefined) {
      throw new Error(`unexpected argument ${args[index]}`);
    }
    const value = args[index + 1] as string;
    const split = value.lastIndexOf("=");
    if (split <= 0) throw new Error(`--map needs from=to, got ${value}`);
    maps.push({ from: value.slice(0, split), to: value.slice(split + 1) });
  }
  return maps;
}

if (import.meta.main) {
  const [command, ...rest] = process.argv.slice(2);
  try {
    if (command === "normalize") {
      const input = readFileSync(0, "utf8");
      process.stdout.write(
        stripNoise(
          normalizeLocations(normalizeExpanded(input, parseMaps(rest))),
        ),
      );
    } else if (command === "summary") {
      const [diffPath, basePath, headPath] = rest;
      if (!diffPath || !basePath || !headPath) {
        throw new Error("usage: summary <diff> <base> <head>");
      }
      const summary = summarizeDiff(
        readFileSync(diffPath, "utf8"),
        readFileSync(basePath, "utf8"),
        readFileSync(headPath, "utf8"),
      );
      console.log(formatSummary(summary));
    } else {
      throw new Error("usage: macos-expanded.ts normalize|summary ...");
    }
  } catch (error) {
    console.error(error instanceof Error ? error.message : String(error));
    process.exit(2);
  }
}
