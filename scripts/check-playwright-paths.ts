// The Playwright workflow runs on a pull request only when a changed file matches the
// `frontend=` regex of its `changes` job. A test that reads a file outside src/ and tests/
// (tests/windowSizes.ts reads src-tauri/src/app_windows.rs) is stale as soon as that file
// changes, so the regex has to match it too: a Rust-only pull request once moved the
// constants out of lib.rs, Playwright was skipped, and main went red.
//
// This reads the regex from the workflow and fails when a file the frontend code or the
// tests refer to is outside it. It looks at: any `src-tauri/...` path written in tests/,
// src/ or the Playwright and Vite configs, and every relative import or
// `new URL("...", import.meta.url)` that resolves outside src/ and tests/.
import { execFileSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { posix } from "node:path";

export const WORKFLOW = ".github/workflows/playwright.yml";
/** Directories whose content every Playwright run is built from, already in the filter. */
const OWN_TREES = ["src/", "tests/"];
const SCANNED =
  /^(src|tests)\/.*\.(ts|tsx|js|mjs|css)$|^(playwright|vite)\.config\.ts$/;

const TAURI_PATH = /src-tauri\/[\w./-]*[\w-]/g;
const RELATIVE_REFERENCE =
  /(?:from\s*|import\s*\(?\s*|new URL\(\s*|glob\(\s*|require\(\s*)["'`](\.[^"'`]*)["'`]/g;

/** The `frontend=` regex of the workflow's path filter. */
export function frontendFilter(workflow: string): RegExp {
  const arg = workflow.match(/'frontend=([^']+)'/);
  if (!arg) throw new Error(`${WORKFLOW}: no 'frontend=...' path filter found`);
  // changed-paths.sh matches the whole path (grep -E -x).
  return new RegExp(`^(?:${arg[1]})$`);
}

function withoutComments(source: string): string {
  return source
    .split("\n")
    .filter((line) => !/^\s*(\/\/|\/\*|\*)/.test(line))
    .join("\n");
}

/** Repo-relative files `source` (at `file`) refers to outside src/ and tests/. */
export function externalReferences(file: string, source: string): string[] {
  const code = withoutComments(source);
  const found = new Set<string>();
  for (const [path] of code.matchAll(TAURI_PATH)) found.add(path);
  for (const [, specifier] of code.matchAll(RELATIVE_REFERENCE)) {
    const target = posix
      .normalize(posix.join(posix.dirname(file), specifier.split("?")[0]))
      .replace(/\/$/, "");
    // A bare directory such as the repository root ("../" from tests/) names no file.
    if (target === "." || target.startsWith("..")) continue;
    if (!OWN_TREES.some((tree) => target.startsWith(tree))) found.add(target);
  }
  return [...found];
}

export function uncovered(
  sources: Map<string, string>,
  filter: RegExp,
): string[] {
  const problems: string[] = [];
  for (const [file, source] of sources) {
    for (const path of externalReferences(file, source)) {
      if (!filter.test(path)) {
        problems.push(
          `${file} reads ${path}, which the Playwright path filter does not match. ` +
            `Add it to 'frontend=' in ${WORKFLOW}.`,
        );
      }
    }
  }
  return problems;
}

function main(): void {
  const root = execFileSync("git", ["rev-parse", "--show-toplevel"], {
    encoding: "utf8",
  }).trim();
  process.chdir(root);
  const files = execFileSync("git", ["ls-files", "-z"], {
    encoding: "utf8",
    maxBuffer: 1 << 26,
  })
    .split("\0")
    .filter((file) => SCANNED.test(file));
  const sources = new Map(files.map((f) => [f, readFileSync(f, "utf8")]));
  const filter = frontendFilter(readFileSync(WORKFLOW, "utf8"));
  const problems = uncovered(sources, filter);
  if (problems.length > 0) {
    console.error(problems.join("\n"));
    process.exit(1);
  }
  console.log(
    `${sources.size} files scanned, every path they read outside src/ and tests/ is in the Playwright path filter`,
  );
}

if (import.meta.main) main();
