/**
 * @fileoverview Offline signature checks against a throw-away key: what a
 * release signed by the real key passes, and what a Mac or Windows client
 * would otherwise be tricked into installing fails.
 */
import assert from "node:assert/strict";
import { mkdtemp, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import { createTestKey } from "./fixtures/minisign-fixture.mjs";
import {
  signedFileNames,
  verifyManifestSignatures,
  verifySignature,
} from "./updater-signature.mjs";

const key = createTestKey();
const otherKey = createTestKey();
const data = Buffer.from("installer bytes");

const fields = verifySignature({
  data,
  signature: key.sign(data, { file: "a.exe", version: "1.13.0" }),
  pubkey: key.pubkey,
});
assert.equal(fields.file, "a.exe");
assert.equal(fields.version, "1.13.0");

assert.throws(
  () =>
    verifySignature({
      data,
      signature: otherKey.sign(data, { file: "a.exe", version: "1.13.0" }),
      pubkey: key.pubkey,
    }),
  /signed by key/,
);
assert.throws(
  () =>
    verifySignature({
      data: Buffer.from("tampered"),
      signature: key.sign(data, { file: "a.exe", version: "1.13.0" }),
      pubkey: key.pubkey,
    }),
  /does not match the file/,
);

const signed = key.sign(data, { file: "a.exe", version: "1.13.0" });
const decoded = Buffer.from(signed, "base64").toString("utf8");
const forgedComment = decoded.replace("version:1.13.0", "version:1.12.0");
assert.throws(
  () =>
    verifySignature({
      data,
      signature: Buffer.from(forgedComment).toString("base64"),
      pubkey: key.pubkey,
    }),
  /trusted comment does not match/,
);
assert.throws(
  () => verifySignature({ data, signature: "not base64!", pubkey: key.pubkey }),
  /not base64/,
);
assert.throws(
  () =>
    verifySignature({
      data,
      signature: Buffer.from("untrusted comment: x\nAAAA\n").toString("base64"),
      pubkey: key.pubkey,
    }),
  /no trusted comment/,
);

assert.deepEqual(
  signedFileNames("windows-x86_64", "Murmur_1.13.0_x64-setup.exe"),
  ["Murmur_1.13.0_x64-setup.exe"],
);
assert.deepEqual(
  signedFileNames("darwin-aarch64", "Murmur_aarch64.app.tar.gz"),
  ["Murmur_aarch64.app.tar.gz", "Murmur.app.tar.gz"],
);

const directory = await mkdtemp(path.join(tmpdir(), "updater-signature-"));
const arm = Buffer.from("arm archive");
const intel = Buffer.from("intel archive");
const exe = Buffer.from("windows installer");
await writeFile(path.join(directory, "Murmur_aarch64.app.tar.gz"), arm);
await writeFile(path.join(directory, "Murmur_x64.app.tar.gz"), intel);
await writeFile(path.join(directory, "Murmur_1.13.0_x64-setup.exe"), exe);

const base =
  "https://github.com/DailyXplorer/Murmur/releases/download/v1.13.0/";
const armSig = key.sign(arm, { file: "Murmur.app.tar.gz", version: "1.13.0" });
const intelSig = key.sign(intel, {
  file: "Murmur.app.tar.gz",
  version: "1.13.0",
});
const exeSig = key.sign(exe, {
  file: "Murmur_1.13.0_x64-setup.exe",
  version: "1.13.0",
});

/**
 * @param {Record<string, string>} signatures Overrides by key.
 * @returns {{platforms: Record<string, {url: string, signature: string}>}}
 */
function manifestWith(signatures = {}) {
  const entries = {
    "darwin-aarch64-app": [armSig, "Murmur_aarch64.app.tar.gz"],
    "darwin-aarch64": [armSig, "Murmur_aarch64.app.tar.gz"],
    "darwin-x86_64-app": [intelSig, "Murmur_x64.app.tar.gz"],
    "darwin-x86_64": [intelSig, "Murmur_x64.app.tar.gz"],
    "windows-x86_64-nsis": [exeSig, "Murmur_1.13.0_x64-setup.exe"],
    "windows-x86_64": [exeSig, "Murmur_1.13.0_x64-setup.exe"],
  };
  return {
    platforms: Object.fromEntries(
      Object.entries(entries).map(([name, [signature, file]]) => [
        name,
        { signature: signatures[name] ?? signature, url: `${base}${file}` },
      ]),
    ),
  };
}

/**
 * @param {ReturnType<typeof manifestWith>} manifest
 * @param {{pubkey?: string, version?: string}} [overrides]
 * @returns {Promise<string[]>}
 */
function verify(manifest, overrides = {}) {
  return verifyManifestSignatures(manifest, {
    directory,
    pubkey: overrides.pubkey ?? key.pubkey,
    version: overrides.version ?? "1.13.0",
  });
}

assert.equal((await verify(manifestWith())).length, 6);

await assert.rejects(
  verify(manifestWith(), { pubkey: otherKey.pubkey }),
  /signed by key/,
);
await assert.rejects(
  verify(manifestWith(), { version: "1.13.1" }),
  /signed version 1\.13\.0 is not 1\.13\.1/,
);
assert.equal((await verify(manifestWith(), { version: "v1.13.0" })).length, 6);

await assert.rejects(
  verify(
    manifestWith({
      "windows-x86_64-nsis": key.sign(exe, {
        file: "Murmur_1.13.0_x64-setup.exe",
        version: "1.12.1",
      }),
    }),
  ),
  /windows-x86_64-nsis: signed version 1\.12\.1/,
);
await assert.rejects(
  verify(
    manifestWith({
      "windows-x86_64": key.sign(exe, { file: "Other.exe", version: "1.13.0" }),
    }),
  ),
  /windows-x86_64: signed file name Other\.exe does not match/,
);
await assert.rejects(
  verify(
    manifestWith({
      "darwin-x86_64-app": key.sign(intel, {
        file: "Murmur_1.13.0_x64-setup.exe",
        version: "1.13.0",
      }),
    }),
  ),
  /darwin-x86_64-app: signed file name/,
);
await assert.rejects(
  verify(
    manifestWith({
      "windows-x86_64": key.sign(exe, { file: "Murmur_1.13.0_x64-setup.exe" }),
    }),
  ),
  /signed version \(none\)/,
);
await assert.rejects(
  verify(manifestWith({ "darwin-aarch64-app": intelSig })),
  /darwin-aarch64-app: Murmur_aarch64\.app\.tar\.gz: signature does not match the file/,
);

const missing = manifestWith();
missing.platforms["windows-x86_64"].url = `${base}Murmur_9.9.9_x64-setup.exe`;
await assert.rejects(
  verify(missing),
  /Murmur_9\.9\.9_x64-setup\.exe is not in/,
);

const traversal = manifestWith();
traversal.platforms["windows-x86_64"].url = `${base}..%2Fescape.exe`;
await assert.rejects(verify(traversal), /not a plain file name/);

await writeFile(path.join(directory, "Murmur_x64.app.tar.gz"), "tampered");
await assert.rejects(
  verify(manifestWith()),
  /darwin-x86_64: .*does not match the file/,
);

console.log("updater-signature: all assertions passed");
