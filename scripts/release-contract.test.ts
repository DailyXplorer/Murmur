import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import path from "node:path";
import {
  checkRepository,
  goldenPath,
  releaseContractProblems,
  type Golden,
} from "./release-contract.ts";

const root = path.resolve(import.meta.dir, "..");
const read = (relative: string) =>
  readFileSync(path.join(root, relative), "utf8");
const releaseYml = read(".github/workflows/release.yml");
const buildYml = read(".github/workflows/build.yml");
const golden = JSON.parse(read(goldenPath)) as Golden;

const problemsFor = (release: string, build = buildYml) =>
  releaseContractProblems(release, build, golden);

/** The text with `from` replaced by `to`, failing when `from` is not there. */
function edit(source: string, from: string, to: string): string {
  assert.ok(source.includes(from), `fixture text is gone: ${from}`);
  return source.replace(from, to);
}

assert.deepEqual(
  checkRepository(root),
  [],
  "the repository keeps the contract",
);

// The golden is the v1.12.1 macOS path: four jobs and the two Mac builds.
assert.deepEqual(Object.keys(golden.release.jobs as object), [
  "validate-release-source",
  "create-release",
  "publish-tauri",
  "publish-release",
]);

const cases: Array<[string, string, RegExp, string?]> = [
  [
    "a Mac job leaves the protected environment",
    edit(releaseYml, 'environment: "release-signing"', 'environment: "other"'),
    /publish-tauri: with changed/,
  ],
  [
    "a Mac signing secret is dropped",
    edit(
      releaseYml,
      "      MACOS_SIGNING_IDENTITY: ${{ secrets.MACOS_SIGNING_IDENTITY }}\n",
      "",
    ),
    /publish-tauri: secrets changed/,
  ],
  [
    "a Windows secret reaches the Mac build",
    edit(
      releaseYml,
      "      MACOS_CERT_PASSWORD: ${{ secrets.MACOS_CERT_PASSWORD }}\n",
      "      MACOS_CERT_PASSWORD: ${{ secrets.MACOS_CERT_PASSWORD }}\n      WINDOWS_CERT: ${{ secrets.WINDOWS_CERT }}\n",
    ),
    /publish-tauri mentions Windows/,
  ],
  [
    "a Mac matrix entry changes",
    edit(
      releaseYml,
      'args: "--target x86_64-apple-darwin"',
      'args: "--target x86_64-apple-darwin --debug"',
    ),
    /publish-tauri: strategy changed/,
  ],
  [
    "the Mac build loses its release dependency",
    edit(
      releaseYml,
      "    needs: [validate-release-source, create-release]\n    strategy:",
      "    needs: [validate-release-source]\n    strategy:",
    ),
    /publish-tauri needs/,
  ],
  [
    "the stable release stops being Latest-gated by the channel",
    edit(
      releaseYml,
      "      - name: Publish release\n        if: inputs.channel == 'stable'\n",
      "      - name: Publish release\n",
    ),
    /Publish release" touches the stable feed or Latest without inputs.channel == 'stable'/,
  ],
  [
    "the test channel makes the release Latest",
    edit(
      releaseYml,
      "              prerelease: true,\n              make_latest: 'false'",
      "              prerelease: true,\n              make_latest: 'true'",
    ),
    /without inputs.channel == 'stable'|makes the release Latest/,
  ],
  [
    "the legacy feed is written in the test channel",
    edit(
      releaseYml,
      "      - name: Point the windows-beta feed at this release\n        if: inputs.channel == 'stable' && inputs.windows\n",
      "      - name: Point the windows-beta feed at this release\n        if: inputs.windows\n",
    ),
    /Point the windows-beta feed at this release" touches the stable feed/,
  ],
  [
    "a skipped Windows job blocks a Mac-only release forever",
    edit(
      releaseYml,
      " || (!inputs.windows && needs.publish-windows.result == 'skipped')",
      "",
    ),
    /accept publish-windows only as success, or skipped/,
  ],
  [
    "the Windows job gets a Mac secret",
    edit(
      releaseYml,
      "    secrets:\n      TAURI_SIGNING_PRIVATE_KEY: ${{ secrets.TAURI_SIGNING_PRIVATE_KEY }}\n      TAURI_SIGNING_PRIVATE_KEY_PASSWORD: ${{ secrets.TAURI_SIGNING_PRIVATE_KEY_PASSWORD }}\n\n  # Runs when",
      "    secrets:\n      TAURI_SIGNING_PRIVATE_KEY: ${{ secrets.TAURI_SIGNING_PRIVATE_KEY }}\n      TAURI_SIGNING_PRIVATE_KEY_PASSWORD: ${{ secrets.TAURI_SIGNING_PRIVATE_KEY_PASSWORD }}\n      APPLE_CERTIFICATE: ${{ secrets.APPLE_CERTIFICATE }}\n\n  # Runs when",
    ),
    /publish-windows passes/,
  ],
  [
    "the release no longer verifies the manifest it publishes",
    edit(
      releaseYml,
      "      - name: Verify public updater manifest\n",
      "      - name: Check nothing\n",
    ),
    /step "Verify public updater manifest" is gone/,
  ],
];

for (const [name, mutated, expected] of cases) {
  const problems = problemsFor(mutated);
  assert.ok(
    problems.some((problem) => expected.test(problem)),
    `${name}: expected ${expected}, got ${JSON.stringify(problems)}`,
  );
}

// build.yml is the pinned macOS build: any edit that is not an action bump shows.
assert.ok(
  problemsFor(
    releaseYml,
    edit(buildYml, 'default: "unsigned-build"', 'default: "release-signing"'),
  ).some((problem) => /build.yml differs/.test(problem)),
);
// An action bump (a new pinned ref) does not.
assert.deepEqual(
  problemsFor(
    releaseYml,
    buildYml.replace(
      /(actions\/checkout@)[0-9a-f]{40}/,
      "$10000000000000000000000000000000000000000",
    ),
  ),
  [],
);

// The test channel only appends the test updater config: nothing else about
// the Mac build arguments or its version check moves for the stable channel.
assert.ok(
  releaseYml.includes("src-tauri/updater-test.conf.json"),
  "the Mac builds take the test updater config",
);

console.log("release contract tests passed");
