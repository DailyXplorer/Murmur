import assert from "node:assert/strict";
import { ciSteps, clippyStep } from "./check.ts";

assert.deepEqual(
  ciSteps.map((step) => [step.command, ...step.args].join(" ")),
  [
    "bun audit",
    "bun run check:translations",
    "bun run check:platforms",
    "bun run test:updater-manifest",
    "bun run test:versions",
    "bun run check:versions",
    "bun run lint",
    "bun run format:check",
    "bun run build",
    "bun run check:types",
  ],
);

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
