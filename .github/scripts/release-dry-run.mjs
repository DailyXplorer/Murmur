/**
 * @fileoverview Release dry run: the unified manifest builder on fake Mac
 * assets and a real Windows installer, offline. Nothing is uploaded and no
 * repository secret is read.
 *
 * `--dir` holds `Murmur_<version>_x64-setup.exe` and its `.sig`, signed with a
 * throw-away key whose public half is `--pubkey-file`. The Mac archives are
 * invented here and signed with another throw-away key (the Mac signature is
 * only copied into the manifest by the builder). The builder runs against
 * fixtures/fake-github.mjs, as in publish-updater-pipeline.test.mjs. Checked:
 * the unified manifest, its Windows signature against the pubkey, the Mac-only
 * manifest of `INCLUDE_WINDOWS=false`, and the legacy Windows feed.
 *
 * Usage: node release-dry-run.mjs --dir <dir> --pubkey-file <file> --version <v>
 */
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { mkdtemp, readFile, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { createTestKey } from "./fixtures/minisign-fixture.mjs";
import { verifyManifestSignatures } from "./updater-signature.mjs";

const here = (name) => fileURLToPath(new URL(name, import.meta.url));
const script = here("./publish-updater-manifest.mjs");
const fakeGithub = here("./fixtures/fake-github.mjs");
const repository = "DailyXplorer/Murmur";

/**
 * @param {string} flag
 * @returns {string}
 */
function option(flag) {
  const index = process.argv.indexOf(flag);
  const value = index === -1 ? undefined : process.argv[index + 1];
  if (!value) throw new Error(`${flag} is required`);
  return value;
}

/**
 * @param {string[]} args
 * @param {{cwd: string, env: Record<string, string>, preload?: boolean}} options
 * @returns {{status: number | null, stdout: string, stderr: string}}
 */
function run(args, { cwd, env, preload = false }) {
  const result = spawnSync(
    process.execPath,
    [...(preload ? ["--import", fakeGithub] : []), script, ...args],
    {
      cwd,
      env: { ...process.env, GITHUB_REPOSITORY: repository, ...env },
      encoding: "utf8",
    },
  );
  return {
    status: result.status,
    stdout: result.stdout,
    stderr: result.stderr,
  };
}

const dir = path.resolve(option("--dir"));
const pubkey = (await readFile(option("--pubkey-file"), "utf8")).trim();
const version = option("--version");
const tag = `v${version}`;
const installer = `Murmur_${version}_x64-setup.exe`;
const installerSignature = (
  await readFile(path.join(dir, `${installer}.sig`), "utf8")
).trim();

const macKey = createTestKey();
const release = {
  release: { tag_name: tag, body: "Dry run" },
  assets: [{ id: 1, name: `Murmur_${version}_aarch64.dmg` }],
  signatures: {},
};
let nextId = 10;
const addAsset = (name, signature) => {
  release.assets.push(
    { id: nextId, name },
    { id: nextId + 1, name: `${name}.sig` },
  );
  release.signatures[nextId + 1] = `${signature}\n`;
  nextId += 2;
};
for (const [name, body] of [
  [`Murmur_${version}_aarch64.app.tar.gz`, "arm64 payload"],
  [`Murmur_${version}_x64.app.tar.gz`, "x86_64 payload"],
]) {
  addAsset(
    name,
    macKey.sign(Buffer.from(body), { file: "Murmur.app.tar.gz", version }),
  );
}
addAsset(installer, installerSignature);

const work = await mkdtemp(path.join(tmpdir(), "release-dry-run-"));
await writeFile(path.join(work, "release.json"), JSON.stringify(release));
const env = {
  FAKE_GITHUB_RELEASE: path.join(work, "release.json"),
  FAKE_GITHUB_UPLOAD: path.join(work, "uploaded.json"),
  GITHUB_TOKEN: "dry-run",
  RELEASE_ID: "1",
  VERSION: version,
  RELEASE_TAG: tag,
};

const unified = run([], { cwd: work, preload: true, env });
assert.equal(unified.status, 0, unified.stderr);
const manifestText = await readFile(path.join(work, "latest.json"), "utf8");
const manifest = JSON.parse(manifestText);
assert.deepEqual(Object.keys(manifest.platforms), [
  "darwin-aarch64",
  "darwin-aarch64-app",
  "darwin-x86_64",
  "darwin-x86_64-app",
  "windows-x86_64",
  "windows-x86_64-nsis",
]);

// The Windows entries are the real installer's: its signature verifies against
// the throw-away public key, with the signed file name and version.
const windowsOnly = {
  platforms: Object.fromEntries(
    Object.entries(manifest.platforms).filter(([key]) =>
      key.startsWith("windows-"),
    ),
  ),
};
await verifyManifestSignatures(windowsOnly, {
  directory: dir,
  pubkey,
  version,
});

// The same manifest is what the public check accepts.
const publicCheck = run(["--verify-public"], {
  cwd: work,
  env: { ...env, MANIFEST: manifestText, INCLUDE_WINDOWS: "true" },
});
assert.equal(publicCheck.status, 0, publicCheck.stderr);

// A Mac is never offered the Windows installer, and a PC never a Mac archive.
for (const key of Object.keys(manifest.platforms)) {
  const { url } = manifest.platforms[key];
  assert.equal(
    key.startsWith("windows-"),
    url.endsWith("-setup.exe"),
    `${key} points at the wrong kind of asset`,
  );
}

// windows=false: the Mac-only release of v1.12.1, with no Windows key.
const macOnly = run([], {
  cwd: work,
  preload: true,
  env: { ...env, INCLUDE_WINDOWS: "false" },
});
assert.equal(macOnly.status, 0, macOnly.stderr);
const macFeed = JSON.parse(
  await readFile(path.join(work, "latest.json"), "utf8"),
);
assert.deepEqual(
  Object.keys(macFeed.platforms).filter((key) => key.startsWith("windows-")),
  [],
);

// The legacy windows-beta pointer: the unified Windows entries only.
await writeFile(path.join(work, "latest.json"), manifestText);
const legacyPath = path.join(work, "legacy.json");
const emitted = run(["--emit-legacy-feed", legacyPath], { cwd: work, env });
assert.equal(emitted.status, 0, emitted.stderr);
const legacyText = await readFile(legacyPath, "utf8");
const legacyCheck = run(["--verify-public-legacy"], {
  cwd: work,
  env: { ...env, MANIFEST: legacyText },
});
assert.equal(legacyCheck.status, 0, legacyCheck.stderr);
const legacy = JSON.parse(legacyText);
assert.deepEqual(Object.keys(legacy.platforms), [
  "windows-x86_64",
  "windows-x86_64-nsis",
]);
assert.equal(
  legacy.platforms["windows-x86_64"].url,
  manifest.platforms["windows-x86_64"].url,
);

// Nothing reached GitHub: the only upload is the fake's file.
console.log(
  `Release dry run passed for ${version}: ${Object.keys(manifest.platforms).length} updater keys, Windows signature verified.`,
);
