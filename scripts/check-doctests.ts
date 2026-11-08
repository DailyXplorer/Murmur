// Pull requests do not run `cargo test --doc` (the `test` and `windows` workflows run it on
// main and by hand only): the crate has no doctest, and the step costs about 94 s, a second
// compile of the crate. This keeps that true. A doc comment may not hold a code block
// rustdoc would run, or the first one would sit unrun until the next push to main.
//
// Rustdoc runs a fenced block unless its info string has `text`, a plain `ignore`, or a word
// that is not a Rust one (```json, ```sh); `no_run` and `compile_fail` still compile and so
// count, and so does `ignore-<target>`, which ignores the block on that target only. It
// also runs an indented block outside a list. Block doc comments (`/** */`) and
// `#[doc = "..."]` carrying a code block are refused outright, as the line scan cannot read
// them. Only src-tauri/src is scanned: `cargo test --doc` reads the library crate, not
// build.rs, tests/, examples/ or the bin's own source. Write such an example as ```text,
// or as a unit test; to run doctests on pull requests again, put the step back in both
// workflows and delete this check.
import { execFileSync } from "node:child_process";
import { readFileSync } from "node:fs";

const RUST_WORDS = new Set([
  "rust",
  "test_harness",
  "compile_fail",
  "should_panic",
  "no_run",
  "standalone_crate",
]);
const DOC_LINE = /^\s*(?:\/\/\/(?!\/)|\/\/!)(.*)$/;
const BLOCK_DOC = /^\s*\/\*[*!](?![*/])/;
const DOC_ATTRIBUTE_WITH_CODE = /#!?\[\s*doc\s*=.*```/;
// An info string after a backtick fence holds no backtick: `/// Use ```x``` here` opens nothing.
const FENCE = /^ {0,3}(?:(`{3,})([^`]*)|(~{3,})(.*))$/;
const LIST_ITEM = /^\s*(?:[-*+]|\d+[.)])\s/;

function runsAsDoctest(info: string): boolean {
  const words = info
    .split(/[\s,]+/)
    .filter(Boolean)
    .map((word) => word.replace(/^\./, ""));
  if (words.includes("ignore") || words.includes("text")) return false;
  if (words.some((word) => word.startsWith("ignore-"))) return true;
  return words.every(
    (word) =>
      RUST_WORDS.has(word) ||
      /^edition\d{4}$/.test(word) ||
      /^E\d{4}$/.test(word),
  );
}

/** `file:line: reason` for each doc-comment code block rustdoc would run. */
export function doctestProblems(file: string, source: string): string[] {
  const problems: string[] = [];
  // The opening fence: its character, and its length.
  let fence: { char: string; length: number } | null = null;
  let previousBlank = false;
  let inList = false;
  let inIndented = false;
  source.split("\n").forEach((raw, index) => {
    const where = `${file}:${index + 1}`;
    if (BLOCK_DOC.test(raw) || DOC_ATTRIBUTE_WITH_CODE.test(raw)) {
      problems.push(
        `${where}: a block doc comment or a #[doc = ...] attribute is not scanned for doctests. Write the documentation with /// lines.`,
      );
    }
    const doc = DOC_LINE.exec(raw);
    if (!doc) {
      fence = null;
      previousBlank = false;
      inList = false;
      inIndented = false;
      return;
    }
    const line = doc[1].startsWith(" ") ? doc[1].slice(1) : doc[1];
    if (fence !== null) {
      // Only the same character, at least as long, and nothing else closes a fence.
      const closing = line.trim();
      if (
        closing.length >= fence.length &&
        closing === fence.char.repeat(closing.length)
      ) {
        fence = null;
        previousBlank = false;
      }
      return;
    }
    const open = FENCE.exec(line);
    if (open) {
      const marker = open[1] ?? open[3];
      const info = (open[2] ?? open[4]).trim();
      fence = { char: marker[0], length: marker.length };
      inIndented = false;
      previousBlank = false;
      if (runsAsDoctest(info)) {
        problems.push(
          `${where}: a doc comment code block (${marker}${info}) is a doctest. Mark it \`text\` or \`ignore\`, or write a unit test.`,
        );
      }
      return;
    }
    if (line.trim() === "") {
      previousBlank = true;
      return;
    }
    const indented = /^ {4,}\S/.test(line);
    if (indented && (previousBlank || inIndented) && !inList) {
      if (!inIndented) {
        problems.push(
          `${where}: an indented block in a doc comment is a doctest. Fence it as \`text\`, or write a unit test.`,
        );
      }
      inIndented = true;
    } else if (!indented) {
      inIndented = false;
      inList = LIST_ITEM.test(line) || (inList && previousBlank === false);
    }
    previousBlank = false;
  });
  return problems;
}

function main(): void {
  const root = execFileSync("git", ["rev-parse", "--show-toplevel"], {
    encoding: "utf8",
  }).trim();
  process.chdir(root);
  const files = execFileSync("git", ["ls-files", "-z", "src-tauri/src"], {
    encoding: "utf8",
    maxBuffer: 1 << 26,
  })
    .split("\0")
    .filter(
      (file) => file.startsWith("src-tauri/src/") && file.endsWith(".rs"),
    );
  const problems = files.flatMap((file) =>
    doctestProblems(file, readFileSync(file, "utf8")),
  );
  if (problems.length > 0) {
    console.error(problems.join("\n"));
    process.exit(1);
  }
  console.log(`${files.length} Rust files, no doctest`);
}

if (import.meta.main) main();
