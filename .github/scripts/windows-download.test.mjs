/**
 * @fileoverview Selection rules for the stable Windows installer link.
 */
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import {
  digestHex,
  isVersionedWindowsInstaller,
  newestWindowsRelease,
  releaseNotes,
  stableDownloadUrl,
  WINDOWS_INSTALLER_ASSET,
  WINDOWS_INSTALLER_TAG,
} from "./windows-download.mjs";

const HASH = "a".repeat(64);

const releases = [
  {
    tag_name: "v1.7.10",
    draft: false,
    published_at: "2026-10-21T21:39:40Z",
    assets: [{ name: "Murmur_1.7.10_aarch64.dmg" }],
  },
  {
    tag_name: "v1.7.7-beta.3",
    draft: false,
    published_at: "2026-10-19T22:23:38Z",
    target_commitish: "abc",
    assets: [
      {
        name: "Murmur_1.7.7-beta.3_x64-setup.exe",
        digest: `sha256:${HASH.toUpperCase()}`,
        size: 9,
        url: "https://api.github.com/repos/DailyXplorer/Murmur/releases/assets/3",
      },
    ],
  },
  {
    tag_name: "v1.7.7-beta.1",
    draft: false,
    published_at: "2026-10-19T20:20:11Z",
    assets: [
      {
        name: "Murmur_1.7.7-beta.1_x64-setup.exe",
        digest: "sha256:old",
        size: 8,
      },
    ],
  },
  {
    tag_name: WINDOWS_INSTALLER_TAG,
    draft: false,
    published_at: "2026-10-21T23:00:00Z",
    assets: [
      {
        name: WINDOWS_INSTALLER_ASSET,
        digest: "sha256:abc",
        size: 9,
      },
    ],
  },
  {
    tag_name: "v9.0.0-draft",
    draft: true,
    published_at: "2026-10-22T00:00:00Z",
    assets: [{ name: "Murmur_9.0.0_x64-setup.exe" }],
  },
];

assert.equal(isVersionedWindowsInstaller(WINDOWS_INSTALLER_ASSET), false);
assert.equal(
  isVersionedWindowsInstaller("Murmur_1.7.7-beta.3_x64-setup.exe"),
  true,
);
assert.equal(digestHex(`sha256:${HASH.toUpperCase()}`), HASH);
assert.equal(digestHex("sha256:ABC"), null);
assert.equal(digestHex("nope"), null);

const chosen = newestWindowsRelease(releases);
assert.equal(chosen.tagName, "v1.7.7-beta.3");
assert.equal(chosen.assetName, "Murmur_1.7.7-beta.3_x64-setup.exe");
assert.equal(chosen.digest, HASH);
assert.equal(
  chosen.assetUrl,
  "https://api.github.com/repos/DailyXplorer/Murmur/releases/assets/3",
);
assert.equal(newestWindowsRelease([]), null);
assert.equal(
  newestWindowsRelease([
    {
      tag_name: "v1.7.10",
      draft: false,
      published_at: "2026-10-21T21:39:40Z",
      assets: [{ name: "latest.json" }],
    },
  ]),
  null,
);

assert.throws(
  () =>
    newestWindowsRelease([
      {
        tag_name: "v1.8.0-beta.1",
        draft: false,
        published_at: "2026-10-22T00:00:00Z",
        assets: [
          { name: "Murmur_1.8.0-beta.1_x64-setup.exe" },
          { name: "Murmur_1.8.0-beta.1-extra_x64-setup.exe" },
        ],
      },
    ]),
  /more than one Windows installer/,
);

const url = stableDownloadUrl("DailyXplorer/Murmur");
assert.equal(
  url,
  "https://github.com/DailyXplorer/Murmur/releases/download/windows-installer/Murmur_x64-setup.exe",
);

const notes = releaseNotes({
  repo: "DailyXplorer/Murmur",
  tagName: "v1.7.7-beta.3",
  assetName: "Murmur_1.7.7-beta.3_x64-setup.exe",
});
assert.match(notes, /v1\.7\.7-beta\.3/);
assert.match(notes, /issues\/157/);
assert.doesNotMatch(notes, /\b(fix(?:es)?|close[sd]?|resolve[sd]?)\b/i);

const readme = readFileSync(
  join(dirname(fileURLToPath(import.meta.url)), "../../README.md"),
  "utf8",
);
assert.ok(readme.includes(url), "README must use the stable Windows download");
assert.equal(readme.includes("1.7.7-beta"), false);
