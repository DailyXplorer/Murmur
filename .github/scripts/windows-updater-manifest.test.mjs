/**
 * @fileoverview The Windows update feed: manifest shape, offline signature
 * check against a throw-away key (made by the Tauri CLI: `tauri signer
 * generate`, then `tauri signer sign --app-version 1.9.0-beta.7` on the
 * fixture installer; the private keys were not kept), and the public checks
 * against a stand-in for GitHub.
 */
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { spawnSync } from "node:child_process";
import { mkdtempSync, readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import {
  assertWindowsManifest,
  assetUrl,
  buildManifest,
  installerName,
  releaseNotes,
  verifyInstaller,
  verifyPublic,
  versionOfTag,
} from "./windows-updater-manifest.mjs";

const here = path.dirname(fileURLToPath(import.meta.url));
const fixtures = path.join(here, "fixtures", "windows-updater");
const NAME = "Murmur_1.9.0-beta.7_x64-setup.exe";
const TAG = "v1.9.0-beta.7";
const REPOSITORY = "DailyXplorer/Murmur";
const installer = readFileSync(path.join(fixtures, NAME));
const signature = readFileSync(path.join(fixtures, `${NAME}.sig`), "utf8");
const pubkey = readFileSync(path.join(fixtures, "signer.pub"), "utf8").trim();
const otherPubkey = readFileSync(
  path.join(fixtures, "other-signer.pub"),
  "utf8",
).trim();

const manifest = () =>
  buildManifest({
    tag: TAG,
    notes: "Notes of the beta.\n",
    signature,
    repository: REPOSITORY,
    pubDate: "2026-11-05T10:00:00.000Z",
  });

/** @param {() => unknown} action @param {RegExp} message */
async function refuses(action, message) {
  await assert.rejects(async () => action(), message);
}

// The tag names the version.
assert.equal(versionOfTag("v1.9.0-beta.7"), "1.9.0-beta.7");
assert.equal(versionOfTag("v1.9.0-beta.10"), "1.9.0-beta.10");
for (const bad of [
  "v1.9.0",
  "1.9.0-beta.7",
  "v1.9.0-beta.",
  "v1.9.0-beta.x",
  "v1.9.0-rc.1",
  "v1.9.0-beta.7-extra",
  "v1.9-beta.7",
  "windows-beta",
  "",
]) {
  assert.throws(() => versionOfTag(bad), /is not vX\.Y\.Z-beta\.N/, bad);
}
assert.equal(installerName("1.9.0-beta.7"), NAME);

// The notes wrap the tag message and end with the checksum verify-public reads.
const notesText = releaseNotes({
  version: "1.9.0-beta.7",
  message: "\nWhat is new in the beta.\n\n",
  checksum: createHash("sha256").update(installer).digest("hex"),
});
assert.match(
  notesText,
  /^## Murmur for Windows 1\.9\.0-beta\.7\n\nWhat is new in the beta\.\n\n### Install without administrator rights/,
);
assert.match(notesText, /Download `Murmur_1\.9\.0-beta\.7_x64-setup\.exe`/);
assert.match(notesText, /SmartScreen/);
assert.match(notesText, /first beta that updates itself/);
assert.match(notesText, /\nSHA-256: [0-9a-f]{64}\n$/);
assert.doesNotMatch(
  releaseNotes({
    version: "1.9.0-beta.7",
    message: "",
    checksum: "0".repeat(64),
  }),
  /\n\n\n/,
);

// The manifest has both Windows keys, the versioned asset and the signature as is.
const built = manifest();
assert.equal(built.version, "1.9.0-beta.7");
assert.equal(built.notes, "Notes of the beta.\n");
assert.equal(built.pub_date, "2026-11-05T10:00:00.000Z");
assert.deepEqual(Object.keys(built.platforms), [
  "windows-x86_64",
  "windows-x86_64-nsis",
]);
for (const entry of Object.values(built.platforms)) {
  assert.equal(
    entry.url,
    `https://github.com/DailyXplorer/Murmur/releases/download/v1.9.0-beta.7/${NAME}`,
  );
  assert.equal(entry.signature, signature.trim());
}
assertWindowsManifest(built, TAG, REPOSITORY);
assert.equal(
  assetUrl(REPOSITORY, "v1.9.0-beta.7", "Murmur 1.9.0 (x64).exe"),
  "https://github.com/DailyXplorer/Murmur/releases/download/v1.9.0-beta.7/Murmur%201.9.0%20(x64).exe",
);

// The feed stays Windows only and well formed.
assert.throws(
  () =>
    assertWindowsManifest(
      {
        ...built,
        platforms: {
          ...built.platforms,
          "darwin-aarch64": built.platforms["windows-x86_64"],
        },
      },
      TAG,
    ),
  /non-Windows updater platforms: darwin-aarch64/,
);
assert.throws(
  () => assertWindowsManifest({ ...built, version: "1.9.0-beta.6" }, TAG),
  /is not 1\.9\.0-beta\.7/,
);
assert.throws(
  () => assertWindowsManifest({ ...built, platforms: {} }, TAG),
  /missing updater platform windows-x86_64/,
);
assert.throws(
  () =>
    assertWindowsManifest(
      {
        ...built,
        platforms: {
          ...built.platforms,
          "windows-x86_64": {
            ...built.platforms["windows-x86_64"],
            signature: "other",
          },
        },
      },
      TAG,
    ),
  /entries differ/,
);
assert.throws(
  () =>
    assertWindowsManifest(
      {
        ...built,
        platforms: Object.fromEntries(
          Object.keys(built.platforms).map((key) => [
            key,
            {
              ...built.platforms[key],
              url: assetUrl(REPOSITORY, "v1.9.0-beta.6", NAME),
            },
          ]),
        ),
      },
      TAG,
      REPOSITORY,
    ),
  /does not point at the v1\.9\.0-beta\.7 installer/,
);

// The signature of the fixture verifies, and every way to break it is refused.
const check = (overrides = {}) =>
  verifyInstaller({
    installer,
    fileName: NAME,
    manifest: built,
    tag: TAG,
    pubkey,
    ...overrides,
  });
check();

await refuses(() => check({ pubkey: otherPubkey }), /was made by key/);
await refuses(
  () => check({ installer: Buffer.concat([installer, Buffer.from("x")]) }),
  /does not match the file/,
);
await refuses(
  () => check({ fileName: "Murmur_1.9.0-beta.7_arm64-setup.exe" }),
  /signature is for Murmur_1\.9\.0-beta\.7_x64-setup\.exe/,
);
await refuses(
  () => check({ tag: "v1.9.0-beta.8" }),
  /signature is for version 1\.9\.0-beta\.7, not 1\.9\.0-beta\.8/,
);

// A trusted comment edited to claim another version breaks its own signature.
const decoded = Buffer.from(signature.trim(), "base64").toString("utf8");
const forged = Buffer.from(
  decoded.replace("version:1.9.0-beta.7", "version:1.9.0-beta.8"),
  "utf8",
).toString("base64");
assert.notEqual(forged, signature.trim());
const forgedManifest = {
  ...built,
  platforms: Object.fromEntries(
    Object.keys(built.platforms).map((key) => [
      key,
      { ...built.platforms[key], signature: forged },
    ]),
  ),
};
await refuses(
  () => check({ manifest: forgedManifest, tag: "v1.9.0-beta.8" }),
  /trusted comment does not match its signature/,
);

// A signature that is not minisign at all.
for (const garbage of [
  "",
  "not base64!",
  Buffer.from("hello").toString("base64"),
]) {
  await refuses(
    () =>
      check({
        manifest: {
          ...built,
          platforms: { "windows-x86_64": { url: "u", signature: garbage } },
        },
      }),
    /signature|base64/,
  );
}

// The public checks, against a stand-in for GitHub.
const checksum = createHash("sha256").update(installer).digest("hex");
const FEED = `https://github.com/${REPOSITORY}/releases/download/windows-beta/latest.json`;
const MACOS_FEED = `https://github.com/${REPOSITORY}/releases/latest/download/latest.json`;
const macosManifest = {
  version: "1.12.0",
  platforms: { "darwin-aarch64": {}, "darwin-aarch64-app": {} },
};
const json = (value) => () =>
  new Response(JSON.stringify(value), { status: 200 });
const world = (changes = {}) => ({
  [FEED]: json(built),
  [built.platforms["windows-x86_64"].url]: () =>
    new Response(installer, { status: 200 }),
  [`https://api.github.com/repos/${REPOSITORY}/releases/tags/${TAG}`]: json({
    prerelease: true,
    body: `Notes.\n\nSHA-256: ${checksum}\n`,
  }),
  [MACOS_FEED]: json(macosManifest),
  [`https://api.github.com/repos/${REPOSITORY}/releases/latest`]: json({
    tag_name: "v1.12.0",
    prerelease: false,
  }),
  ...changes,
});
const publicCheck = (changes = {}, overrides = {}) => {
  const served = world(changes);
  return verifyPublic({
    tag: TAG,
    repository: REPOSITORY,
    pubkey,
    fetchImpl: async (url) =>
      served[url]?.() ?? new Response("missing", { status: 404 }),
    ...overrides,
  });
};

await publicCheck();
await refuses(
  () => publicCheck({ [FEED]: () => new Response("", { status: 404 }) }),
  /answered 404/,
);
await refuses(
  () => publicCheck({}, { pubkey: otherPubkey }),
  /was made by key/,
);
await refuses(
  () =>
    publicCheck({
      [FEED]: json({ ...built, version: "1.9.0-beta.6" }),
    }),
  /is not 1\.9\.0-beta\.7/,
);
await refuses(
  () =>
    publicCheck({
      [`https://api.github.com/repos/${REPOSITORY}/releases/tags/${TAG}`]: json(
        {
          prerelease: true,
          body: `SHA-256: ${"0".repeat(64)}`,
        },
      ),
    }),
  /SHA-256 in the release notes/,
);
await refuses(
  () =>
    publicCheck({
      [`https://api.github.com/repos/${REPOSITORY}/releases/tags/${TAG}`]: json(
        {
          prerelease: false,
          body: `SHA-256: ${checksum}`,
        },
      ),
    }),
  /is not a pre-release/,
);
await refuses(
  () =>
    publicCheck({
      [MACOS_FEED]: json({
        ...macosManifest,
        platforms: { ...macosManifest.platforms, "windows-x86_64": {} },
      }),
    }),
  /macOS feed lists windows-x86_64/,
);
await refuses(
  () =>
    publicCheck({
      [MACOS_FEED]: json({ ...macosManifest, version: TAG.slice(1) }),
    }),
  /macOS feed announces 1\.9\.0-beta\.7/,
);
await refuses(
  () =>
    publicCheck({
      [`https://api.github.com/repos/${REPOSITORY}/releases/latest`]: json({
        tag_name: TAG,
        prerelease: true,
      }),
    }),
  /latest release is v1\.9\.0-beta\.7/,
);

// The command line: build, then verify against a config holding the pubkey.
const dir = mkdtempSync(path.join(tmpdir(), "windows-updater-"));
const config = path.join(dir, "tauri.conf.json");
const notes = path.join(dir, "notes.md");
const out = path.join(dir, "latest.json");
writeFileSync(config, JSON.stringify({ plugins: { updater: { pubkey } } }));
writeFileSync(notes, "Notes of the beta.\n");
const script = path.join(here, "windows-updater-manifest.mjs");
const run = (...args) =>
  spawnSync(process.execPath, [script, ...args], { encoding: "utf8" });

const built2 = run(
  "build",
  "--installer",
  path.join(fixtures, NAME),
  "--sig",
  path.join(fixtures, `${NAME}.sig`),
  "--tag",
  TAG,
  "--notes",
  notes,
  "--repository",
  REPOSITORY,
  "--out",
  out,
);
assert.equal(built2.status, 0, built2.stderr);
assert.equal(JSON.parse(readFileSync(out, "utf8")).version, "1.9.0-beta.7");

const verified = run(
  "verify",
  "--installer",
  path.join(fixtures, NAME),
  "--manifest",
  out,
  "--tag",
  TAG,
  "--config",
  config,
);
assert.equal(verified.status, 0, verified.stderr);

writeFileSync(
  config,
  JSON.stringify({ plugins: { updater: { pubkey: otherPubkey } } }),
);
const refused = run(
  "verify",
  "--installer",
  path.join(fixtures, NAME),
  "--manifest",
  out,
  "--tag",
  TAG,
  "--config",
  config,
);
assert.equal(refused.status, 1);
assert.match(refused.stderr, /was made by key/);

const badTag = run(
  "build",
  "--installer",
  path.join(fixtures, NAME),
  "--sig",
  path.join(fixtures, `${NAME}.sig`),
  "--tag",
  "v1.9.0",
  "--notes",
  notes,
  "--repository",
  REPOSITORY,
  "--out",
  out,
);
assert.equal(badTag.status, 1);
assert.match(badTag.stderr, /is not vX\.Y\.Z-beta\.N/);

const notesOut = path.join(dir, "release-notes.md");
const wrote = run(
  "notes",
  "--version",
  "1.9.0-beta.7",
  "--message",
  notes,
  "--installer",
  path.join(fixtures, NAME),
  "--out",
  notesOut,
);
assert.equal(wrote.status, 0, wrote.stderr);
assert.match(
  readFileSync(notesOut, "utf8"),
  new RegExp(`\\nSHA-256: ${checksum}\\n$`),
);

console.log("windows updater manifest tests passed");
