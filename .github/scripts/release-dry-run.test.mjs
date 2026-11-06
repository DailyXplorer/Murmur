/**
 * @fileoverview The release dry run accepts a signed Windows installer and
 * refuses one whose file or signing key does not match (offline).
 */
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { mkdtemp, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { createTestKey } from "./fixtures/minisign-fixture.mjs";

const dryRun = fileURLToPath(new URL("./release-dry-run.mjs", import.meta.url));
const version = "1.13.0";
const installer = `Murmur_${version}_x64-setup.exe`;

/**
 * @param {{key?: ReturnType<typeof createTestKey>, signedKey?: ReturnType<typeof createTestKey>, bytes?: string, signedBytes?: string}} [options]
 * @returns {Promise<{status: number | null, stderr: string, stdout: string}>}
 */
async function dryRunWith({
  key = createTestKey(),
  signedKey = key,
  bytes = "installer",
  signedBytes = bytes,
} = {}) {
  const dir = await mkdtemp(path.join(tmpdir(), "dry-run-test-"));
  await writeFile(path.join(dir, installer), bytes);
  await writeFile(
    path.join(dir, `${installer}.sig`),
    signedKey.sign(Buffer.from(signedBytes), { file: installer, version }),
  );
  const pubkeyFile = path.join(dir, "key.pub");
  await writeFile(pubkeyFile, `${key.pubkey}\n`);
  return spawnSync(
    process.execPath,
    [dryRun, "--dir", dir, "--pubkey-file", pubkeyFile, "--version", version],
    { encoding: "utf8" },
  );
}

const good = await dryRunWith();
assert.equal(good.status, 0, good.stderr);
assert.match(good.stdout, /6 updater keys, Windows signature verified/);

const tampered = await dryRunWith({ signedBytes: "another installer" });
assert.notEqual(tampered.status, 0);
assert.match(tampered.stderr, /windows-x86_64/);

const otherKey = await dryRunWith({ signedKey: createTestKey() });
assert.notEqual(otherKey.status, 0);

console.log("release dry run tests passed");
