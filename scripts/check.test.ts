import assert from "node:assert/strict";
import { ciSteps, clippyStep, ignoredAdvisories } from "./check.ts";

assert.deepEqual(
  ciSteps.map((step) => [step.command, ...step.args].join(" ")),
  [
    "bun audit --ignore GHSA-fc5q-39xc-vjpf",
    "bun run check:translations",
    "bun run check:platforms",
    "bun run test:platforms",
    "bun run check:file-size",
    "bun run test:file-size",
    "bun run check:playwright-paths",
    "bun run test:playwright-paths",
    "bun run check:doctests",
    "bun run test:doctests",
    "bun run test:updater-manifest",
    "bun run test:release-contract",
    "bun run test:changed-paths",
    "bun run test:versions",
    "bun run test:macos-expanded",
    "bun run check:versions",
    "bun run lint",
    "bun run format:check",
    "bun run build",
    "bun run check:types",
  ],
);

// An ignored advisory says why it cannot be fixed yet.
for (const { id, why } of ignoredAdvisories) {
  assert.match(id, /^GHSA(-[0-9a-z]{4}){3}$/);
  assert.ok(why.length > 40, `${id} needs its reason`);
}

const clippy = clippyStep();
assert.equal(clippy.command, "cargo");
assert.equal(clippy.cwd, "src-tauri");
assert.deepEqual(clippy.args, [
  "clippy",
  "--locked",
  "--all-targets",
  "--all-features",
  "--",
  "-D",
  "warnings",
]);

console.log("check contract passed");
