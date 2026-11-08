import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import {
  externalReferences,
  frontendFilter,
  uncovered,
  WORKFLOW,
} from "./check-playwright-paths.ts";

const workflow = `
        run: |
          ./.github/scripts/changed-paths.sh \\
            'frontend=src/.*|tests/.*|src-tauri/src/app_windows\\.rs|package\\.json'
`;
const filter = frontendFilter(workflow);
assert.equal(filter.test("src-tauri/src/app_windows.rs"), true);
assert.equal(filter.test("src-tauri/src/app_windows.rs.bak"), false);
assert.equal(filter.test("src-tauri/src/lib.rs"), false);
assert.throws(() => frontendFilter("run: echo"), /no 'frontend=/);

// The read that went stale: a URL from tests/ into src-tauri.
const windowSizes = `
const libRs = readFileSync(
  new URL("../src-tauri/src/app_windows.rs", import.meta.url),
  "utf8",
);`;
assert.deepEqual(externalReferences("tests/windowSizes.ts", windowSizes), [
  "src-tauri/src/app_windows.rs",
]);
assert.deepEqual(
  uncovered(new Map([["tests/windowSizes.ts", windowSizes]]), filter),
  [],
);

// A new read of a src-tauri file the filter lacks fails; comments do not count.
const newRead = `readFileSync(new URL("../src-tauri/src/overlay.rs", import.meta.url))`;
assert.equal(
  uncovered(new Map([["tests/overlay.spec.ts", newRead]]), filter).length,
  1,
);
assert.deepEqual(
  uncovered(
    new Map([
      ["tests/a.ts", "// see src-tauri/src/overlay.rs\n * src-tauri/x.rs"],
    ]),
    filter,
  ),
  [],
);

// Relative imports: inside src/ and tests/ are fine, outside need the filter.
assert.deepEqual(
  externalReferences(
    "tests/fixtures/a.tsx",
    `import x from "../../src/bindings";\nimport y from "../shared";\nimport "./b";`,
  ),
  [],
);
assert.deepEqual(
  externalReferences("src/a.ts", `import n from "../scripts/names.json?raw";`),
  ["scripts/names.json"],
);
assert.deepEqual(
  externalReferences("src/a.ts", `import n from "../package.json";`),
  ["package.json"],
);
assert.equal(
  uncovered(
    new Map([["src/a.ts", `import n from "../CHANGELOG.md?raw";`]]),
    filter,
  ).length,
  1,
);
// The repository root names no file.
assert.deepEqual(
  externalReferences("tests/s.spec.ts", `new URL("../", import.meta.url)`),
  [],
);

// The real workflow has a parsable filter that covers the one src-tauri file tests read.
assert.equal(
  frontendFilter(readFileSync(WORKFLOW, "utf8")).test(
    "src-tauri/src/app_windows.rs",
  ),
  true,
);

console.log("playwright path check tests passed");
