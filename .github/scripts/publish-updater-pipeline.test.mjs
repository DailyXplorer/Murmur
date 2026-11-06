/**
 * @fileoverview The publish script end to end, offline (fixtures/fake-github.mjs
 * stands in for GitHub): the macOS-only golden file, the unified release, the
 * signature check on downloaded files and the legacy Windows feed.
 */
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { mkdir, mkdtemp, readFile, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { createTestKey } from "./fixtures/minisign-fixture.mjs";

const here = (name) => fileURLToPath(new URL(name, import.meta.url));
const scriptPath = here("./publish-updater-manifest.mjs");
const fakeGithub = here("./fixtures/fake-github.mjs");

/**
 * @param {string[]} args CLI flags.
 * @param {{cwd: string, env?: Record<string, string>, preload?: boolean}} options
 * @returns {{status: number | null, stdout: string, stderr: string}}
 */
function runScript(args, { cwd, env = {}, preload = false }) {
  const base = { ...process.env, INCLUDE_WINDOWS: "", GITHUB_REPOSITORY: "" };
  const result = spawnSync(
    process.execPath,
    [...(preload ? ["--import", fakeGithub] : []), scriptPath, ...args],
    { cwd, env: { ...base, ...env }, encoding: "utf8" },
  );
  return {
    status: result.status,
    stdout: result.stdout,
    stderr: result.stderr,
  };
}

/**
 * Runs the publish step against a described release.
 * @param {object} release Fixture shaped as fixtures/fake-github.mjs reads it.
 * @param {Record<string, string>} env
 */
async function publishOffline(release, env) {
  const cwd = await mkdtemp(path.join(tmpdir(), "publish-manifest-"));
  const releasePath = path.join(cwd, "release.json");
  await writeFile(releasePath, JSON.stringify(release));
  const result = runScript([], {
    cwd,
    preload: true,
    env: {
      FAKE_GITHUB_RELEASE: releasePath,
      FAKE_GITHUB_UPLOAD: path.join(cwd, "uploaded.json"),
      FAKE_GITHUB_NOW: "2026-11-06T12:00:00.000Z",
      GITHUB_TOKEN: "token",
      GITHUB_REPOSITORY: "DailyXplorer/Murmur",
      RELEASE_ID: "7",
      ...env,
    },
  });
  const read = (name) => readFile(path.join(cwd, name)).catch(() => null);
  return {
    ...result,
    cwd,
    manifest: await read("latest.json"),
    uploaded: await read("uploaded.json"),
  };
}

// macOS-only release: byte for byte what the script produced before Windows
// existed (the golden file came from the unmodified script on these assets).
const macRelease = JSON.parse(
  await readFile(here("./fixtures/release-v1.12.1-macos-only.json"), "utf8"),
);
const golden = await readFile(
  here("./fixtures/latest-v1.12.1-macos-only.golden.json"),
);
const macOnly = await publishOffline(macRelease, {
  VERSION: "1.12.1",
  INCLUDE_WINDOWS: "false",
});
assert.equal(macOnly.status, 0, macOnly.stderr);
assert.ok(
  macOnly.manifest.equals(golden),
  "mac-only latest.json equals golden",
);
assert.ok(macOnly.uploaded.equals(golden), "uploaded body equals golden");

// A stray Windows installer does not change a macOS-only release.
const withStrayExe = structuredClone(macRelease);
withStrayExe.assets.push(
  { id: 107, name: "Murmur_1.12.1_x64-setup.exe" },
  { id: 108, name: "Murmur_1.12.1_x64-setup.exe.sig" },
);
withStrayExe.signatures["108"] = "c3RyYXkK";
const stray = await publishOffline(withStrayExe, {
  VERSION: "1.12.1",
  INCLUDE_WINDOWS: "false",
});
assert.equal(stray.status, 0, stray.stderr);
assert.ok(stray.manifest.equals(golden));

// Windows expected (the default) but not built: refuse and upload nothing.
for (const windows of [undefined, "true"]) {
  const missingExe = await publishOffline(macRelease, {
    VERSION: "1.12.1",
    ...(windows ? { INCLUDE_WINDOWS: windows } : {}),
  });
  assert.notEqual(missingExe.status, 0);
  assert.match(missingExe.stderr, /Missing updater for windows-x86_64/);
  assert.equal(missingExe.uploaded, null);
}
const badFlag = await publishOffline(macRelease, {
  VERSION: "1.12.1",
  INCLUDE_WINDOWS: "maybe",
});
assert.notEqual(badFlag.status, 0);
assert.match(badFlag.stderr, /INCLUDE_WINDOWS must be true or false/);

// Unified release, signed with a throw-away key.
const key = createTestKey();
const files = {
  "Murmur_aarch64.app.tar.gz": {
    data: Buffer.from("arm"),
    signed: "Murmur.app.tar.gz",
  },
  "Murmur_x64.app.tar.gz": {
    data: Buffer.from("intel"),
    signed: "Murmur.app.tar.gz",
  },
  "Murmur_1.13.0_x64-setup.exe": {
    data: Buffer.from("installer"),
    signed: "Murmur_1.13.0_x64-setup.exe",
  },
};
const unifiedRelease = {
  release: { tag_name: "v1.13.0", body: "Notes" },
  assets: [{ id: 900, name: "Murmur_1.13.0_x64.dmg" }],
  signatures: {},
};
let nextId = 200;
for (const [name, { data, signed }] of Object.entries(files)) {
  const signature = key.sign(data, { file: signed, version: "1.13.0" });
  unifiedRelease.assets.push(
    { id: nextId, name },
    { id: nextId + 1, name: `${name}.sig` },
  );
  unifiedRelease.signatures[nextId + 1] = `${signature}\n`;
  nextId += 2;
}

const unified = await publishOffline(unifiedRelease, { VERSION: "1.13.0" });
assert.equal(unified.status, 0, unified.stderr);
assert.ok(unified.uploaded.equals(unified.manifest));
const unifiedManifest = JSON.parse(unified.manifest);
assert.deepEqual(Object.keys(unifiedManifest.platforms), [
  "darwin-aarch64",
  "darwin-aarch64-app",
  "darwin-x86_64",
  "darwin-x86_64-app",
  "windows-x86_64",
  "windows-x86_64-nsis",
]);
const releaseUrl =
  "https://github.com/DailyXplorer/Murmur/releases/download/v1.13.0/";
assert.equal(
  unifiedManifest.platforms["windows-x86_64"].url,
  `${releaseUrl}Murmur_1.13.0_x64-setup.exe`,
);
assert.equal(
  unifiedManifest.platforms["darwin-x86_64"].url,
  `${releaseUrl}Murmur_x64.app.tar.gz`,
);
assert.equal(
  unifiedManifest.platforms["windows-x86_64"].signature,
  unifiedManifest.platforms["windows-x86_64-nsis"].signature,
);

// The same release with INCLUDE_WINDOWS=false has the four Mac keys only.
const unifiedMacOnly = await publishOffline(unifiedRelease, {
  VERSION: "1.13.0",
  INCLUDE_WINDOWS: "false",
});
assert.equal(unifiedMacOnly.status, 0, unifiedMacOnly.stderr);
const macOnlyFeed = JSON.parse(unifiedMacOnly.manifest);
assert.equal(
  Object.keys(macOnlyFeed.platforms).filter((name) =>
    name.startsWith("windows-"),
  ).length,
  0,
);

// --verify-signatures on the downloaded assets.
const { cwd } = unified;
const manifestFlag = ["--manifest", path.join(cwd, "latest.json")];
const downloads = path.join(cwd, "downloads");
const config = path.join(cwd, "tauri.conf.json");
await mkdir(downloads);
for (const [name, { data }] of Object.entries(files)) {
  await writeFile(path.join(downloads, name), data);
}
const writeConfig = (pubkey) =>
  writeFile(config, JSON.stringify({ plugins: { updater: { pubkey } } }));
const verifySignatures = (env = {}) =>
  runScript(
    ["--verify-signatures", downloads, ...manifestFlag, "--config", config],
    { cwd, env: { VERSION: "1.13.0", ...env } },
  );

await writeConfig(key.pubkey);
const verified = verifySignatures();
assert.equal(verified.status, 0, verified.stderr);
assert.match(verified.stdout, /Verified 6 updater signatures for 1\.13\.0/);
assert.notEqual(verifySignatures({ VERSION: "1.13.1" }).status, 0);
assert.notEqual(verifySignatures({ VERSION: "" }).status, 0);

await writeConfig(createTestKey().pubkey);
const wrongKey = verifySignatures();
assert.notEqual(wrongKey.status, 0);
assert.match(wrongKey.stderr, /signed by key/);

await writeConfig(key.pubkey);
await writeFile(
  path.join(downloads, "Murmur_1.13.0_x64-setup.exe"),
  "tampered",
);
const tampered = verifySignatures();
assert.notEqual(tampered.status, 0);
assert.match(tampered.stderr, /does not match the file/);

// Legacy `windows-beta` feed: same installer and signature, Windows keys only.
const legacyPath = path.join(cwd, "legacy.json");
const emitted = runScript(["--emit-legacy-feed", legacyPath, ...manifestFlag], {
  cwd,
  env: { VERSION: "1.13.0" },
});
assert.equal(emitted.status, 0, emitted.stderr);
const legacy = JSON.parse(await readFile(legacyPath, "utf8"));
assert.deepEqual(Object.keys(legacy.platforms), [
  "windows-x86_64",
  "windows-x86_64-nsis",
]);
assert.deepEqual(
  legacy.platforms["windows-x86_64"],
  unifiedManifest.platforms["windows-x86_64"],
);
assert.equal(legacy.version, "1.13.0");
assert.equal(legacy.notes, unifiedManifest.notes);
assert.equal(legacy.pub_date, unifiedManifest.pub_date);

const macOnlyPath = path.join(unifiedMacOnly.cwd, "latest.json");
assert.notEqual(
  runScript(["--emit-legacy-feed", legacyPath, "--manifest", macOnlyPath], {
    cwd,
    env: { VERSION: "1.13.0" },
  }).status,
  0,
  "a Mac-only release has no legacy feed",
);

/**
 * @param {string} flag
 * @param {unknown} feed
 * @param {string} version
 * @param {Record<string, string>} [env]
 * @returns {number | null} The exit status.
 */
const verifyFeed = (flag, feed, version, env = {}) =>
  runScript([flag], {
    cwd,
    env: { VERSION: version, MANIFEST: JSON.stringify(feed), ...env },
  }).status;

assert.equal(verifyFeed("--verify-public-legacy", legacy, "1.13.0"), 0);
assert.notEqual(verifyFeed("--verify-public-legacy", legacy, "1.13.1"), 0);
assert.notEqual(
  verifyFeed("--verify-public-legacy", unifiedManifest, "1.13.0"),
  0,
);
assert.notEqual(verifyFeed("--verify-public", legacy, "1.13.0"), 0);

// `--verify-public` on the unified feed: Windows required by default.
assert.equal(verifyFeed("--verify-public", unifiedManifest, "1.13.0"), 0);
const pins = { GITHUB_REPOSITORY: "DailyXplorer/Murmur" };
assert.equal(
  verifyFeed("--verify-public", unifiedManifest, "1.13.0", {
    ...pins,
    RELEASE_TAG: "v1.13.0",
  }),
  0,
);
assert.notEqual(
  verifyFeed("--verify-public", unifiedManifest, "1.13.0", {
    ...pins,
    RELEASE_TAG: "v1.12.1",
  }),
  0,
);
assert.notEqual(
  verifyFeed("--verify-public", unifiedManifest, "1.13.0", {
    INCLUDE_WINDOWS: "false",
  }),
  0,
  "a Windows key in a Mac-only release is refused",
);
assert.equal(
  verifyFeed("--verify-public", macOnlyFeed, "1.13.0", {
    INCLUDE_WINDOWS: "false",
  }),
  0,
);
assert.notEqual(verifyFeed("--verify-public", macOnlyFeed, "1.13.0"), 0);

console.log("publish-updater-pipeline: all assertions passed");
