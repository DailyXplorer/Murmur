import assert from "node:assert/strict";
import { versionProblems, type VersionSources } from "./check-versions.ts";

const matching: VersionSources = {
  packageJson: JSON.stringify({ version: "1.2.3" }),
  cargoToml:
    '[package]\nname = "murmur"\nversion = "1.2.3"\n\n[dependencies]\n',
  cargoLock: '[[package]]\nname = "murmur"\nversion = "1.2.3"\n',
  tauriConf: JSON.stringify({ version: "1.2.3" }),
  releaseNoteNames: ["1.2.3.md", "README.md"],
};

assert.deepEqual(versionProblems(matching), []);

assert.deepEqual(
  versionProblems({
    ...matching,
    packageJson: JSON.stringify({ version: "9.9.9" }),
  }).some((problem) => problem.startsWith("versions differ")),
  true,
);

assert.deepEqual(
  versionProblems({
    ...matching,
    releaseNoteNames: ["README.md"],
  }),
  ["missing release notes src/content/release-notes/1.2.3.md"],
);

assert.deepEqual(
  versionProblems({
    ...matching,
    cargoLock: '[[package]]\nname = "other"\nversion = "1.2.3"\n',
  }).some((problem) => problem.includes("Cargo.lock")),
  true,
);

assert.deepEqual(
  versionProblems({
    ...matching,
    tauriConf: "{",
  }).some((problem) => problem.includes("tauri.conf.json")),
  true,
);

// A dependency version inside Cargo.toml must not be read as the package version.
assert.deepEqual(
  versionProblems({
    ...matching,
    cargoToml:
      '[package]\nname = "murmur"\nversion = "1.2.3"\n\n[dependencies]\nserde = { version = "1" }\n',
  }),
  [],
);

console.log("version checks passed");
