/**
 * @fileoverview Dry-run coverage for darwin-only updater mapping, aliasing, and public manifest checks.
 */
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import {
  assertPublicDarwinManifest,
  finalizeDarwinPlatforms,
  updaterTarget,
} from "./publish-updater-manifest.mjs";

const armSig = "Murmur_aarch64.app.tar.gz.sig";
const intelSig = "Murmur_x64.app.tar.gz.sig";

assert.deepEqual(updaterTarget(armSig), {
  os: "darwin",
  arch: "aarch64",
  bundle: "app",
});
assert.deepEqual(updaterTarget(intelSig), {
  os: "darwin",
  arch: "x86_64",
  bundle: "app",
});
assert.deepEqual(updaterTarget("Murmur_aarch64.app.tar.gz.SIG"), {
  os: "darwin",
  arch: "aarch64",
  bundle: "app",
});

assert.equal(updaterTarget("Murmur_1.0.3_aarch64.AppImage.sig"), null);
assert.equal(updaterTarget("Murmur_1.0.3_amd64.AppImage.sig"), null);
assert.equal(updaterTarget("Murmur_1.0.3_amd64.deb.sig"), null);
assert.equal(updaterTarget("Murmur_1.0.3_arm64.deb.sig"), null);
assert.equal(updaterTarget("Murmur-1.0.3-1.aarch64.rpm.sig"), null);
assert.equal(updaterTarget("Murmur-1.0.3-1.x86_64.rpm.sig"), null);
assert.equal(updaterTarget("Murmur_1.0.3_arm64-setup.exe.sig"), null);
assert.equal(updaterTarget("Murmur_1.0.3_x64-setup.exe.sig"), null);
assert.equal(updaterTarget("Murmur_1.0.3_arm64_en-US.msi.sig"), null);
assert.equal(updaterTarget("Murmur_1.0.3_x64_en-US.msi.sig"), null);
assert.equal(updaterTarget("Murmur_aarch64.app.tar.gz"), null);
assert.equal(updaterTarget("Murmur_1.0.3_aarch64.dmg"), null);

const leftoverSigs = [
  "Murmur_1.0.3_aarch64.AppImage.sig",
  "Murmur_1.0.3_amd64.deb.sig",
  "Murmur-1.0.3-1.x86_64.rpm.sig",
  "Murmur_1.0.3_x64-setup.exe.sig",
  "Murmur_1.0.3_x64_en-US.msi.sig",
  armSig,
  intelSig,
];
const mapped = leftoverSigs.map((name) => updaterTarget(name)).filter(Boolean);
assert.deepEqual(
  mapped.map((target) => `${target.os}-${target.arch}-${target.bundle}`).sort(),
  ["darwin-aarch64-app", "darwin-x86_64-app"],
);

const arm = {
  signature: "arm-minisign",
  url: "https://github.com/DailyXplorer/Murmur/releases/download/v1.0.3/Murmur_aarch64.app.tar.gz",
};
const intel = {
  signature: "intel-minisign",
  url: "https://github.com/DailyXplorer/Murmur/releases/download/v1.0.3/Murmur_x64.app.tar.gz",
};

const platforms = {
  "darwin-aarch64-app": arm,
  "darwin-x86_64-app": intel,
};
finalizeDarwinPlatforms(platforms);
assert.equal(platforms["darwin-aarch64"], arm);
assert.equal(platforms["darwin-x86_64"], intel);
assert.equal(platforms["darwin-aarch64-app"], arm);
assert.equal(platforms["darwin-x86_64-app"], intel);
assert.equal(
  Object.keys(platforms).filter(
    (key) => key.startsWith("linux-") || key.startsWith("windows-"),
  ).length,
  0,
);

assert.throws(
  () => finalizeDarwinPlatforms({ "darwin-x86_64-app": intel }),
  /darwin-aarch64/,
);
assert.throws(
  () => finalizeDarwinPlatforms({ "darwin-aarch64-app": arm }),
  /darwin-x86_64/,
);

const manifest = {
  version: "1.0.3",
  platforms: {
    "darwin-aarch64": arm,
    "darwin-aarch64-app": arm,
    "darwin-x86_64": intel,
    "darwin-x86_64-app": intel,
  },
};
assertPublicDarwinManifest(manifest, "1.0.3");

assert.throws(
  () =>
    assertPublicDarwinManifest(
      { version: "1.0.3", platforms: { "darwin-x86_64": intel } },
      "1.0.3",
    ),
  /darwin-aarch64/,
);
assert.throws(
  () =>
    assertPublicDarwinManifest(
      {
        version: "1.0.3",
        platforms: {
          ...manifest.platforms,
          "linux-x86_64-appimage": intel,
        },
      },
      "1.0.3",
    ),
  /Non-darwin/,
);
assert.throws(
  () =>
    assertPublicDarwinManifest(
      {
        version: "1.0.3",
        platforms: {
          "darwin-aarch64": { url: arm.url },
          "darwin-aarch64-app": arm,
          "darwin-x86_64": intel,
          "darwin-x86_64-app": intel,
        },
      },
      "1.0.3",
    ),
  /darwin-aarch64/,
);

const v103Assets = [
  "latest.json",
  "Murmur-1.0.3-1.aarch64.rpm",
  "Murmur-1.0.3-1.aarch64.rpm.sig",
  "Murmur-1.0.3-1.x86_64.rpm",
  "Murmur-1.0.3-1.x86_64.rpm.sig",
  "Murmur_1.0.3_aarch64.AppImage",
  "Murmur_1.0.3_aarch64.AppImage.sig",
  "Murmur_1.0.3_aarch64.dmg",
  "Murmur_1.0.3_amd64.AppImage",
  "Murmur_1.0.3_amd64.AppImage.sig",
  "Murmur_1.0.3_amd64.deb",
  "Murmur_1.0.3_amd64.deb.sig",
  "Murmur_1.0.3_arm64-setup.exe",
  "Murmur_1.0.3_arm64-setup.exe.sig",
  "Murmur_1.0.3_arm64.deb",
  "Murmur_1.0.3_arm64.deb.sig",
  "Murmur_1.0.3_arm64_en-US.msi",
  "Murmur_1.0.3_arm64_en-US.msi.sig",
  "Murmur_1.0.3_x64-setup.exe",
  "Murmur_1.0.3_x64-setup.exe.sig",
  "Murmur_1.0.3_x64.dmg",
  "Murmur_1.0.3_x64_en-US.msi",
  "Murmur_1.0.3_x64_en-US.msi.sig",
  "Murmur_aarch64.app.tar.gz",
  "Murmur_aarch64.app.tar.gz.sig",
  "Murmur_x64.app.tar.gz",
  "Murmur_x64.app.tar.gz.sig",
];
const fromRelease = {};
for (const name of v103Assets) {
  const target = updaterTarget(name);
  if (!target) continue;
  fromRelease[`${target.os}-${target.arch}-${target.bundle}`] = {
    signature: `${name}-signature`,
    url: `https://github.com/DailyXplorer/Murmur/releases/download/v1.0.3/${name.slice(0, -4)}`,
  };
}
finalizeDarwinPlatforms(fromRelease);
assert.deepEqual(Object.keys(fromRelease).sort(), [
  "darwin-aarch64",
  "darwin-aarch64-app",
  "darwin-x86_64",
  "darwin-x86_64-app",
]);
assertPublicDarwinManifest(
  { version: "1.0.3", platforms: fromRelease },
  "1.0.3",
);
assert.match(
  fromRelease["darwin-aarch64"].url,
  /Murmur_aarch64\.app\.tar\.gz$/,
);
assert.match(fromRelease["darwin-x86_64"].url, /Murmur_x64\.app\.tar\.gz$/);

/**
 * Runs the `--verify-public` CLI against a JSON payload via VERSION and MANIFEST.
 * @param {string | undefined} version
 * @param {object | undefined} manifest
 * @returns {{status: number | null, stderr: string}}
 */
function runVerifyPublic(version, manifest) {
  const env = { ...process.env };
  if (version === undefined) delete env.VERSION;
  else env.VERSION = version;
  if (manifest === undefined) delete env.MANIFEST;
  else env.MANIFEST = JSON.stringify(manifest);
  const result = spawnSync(
    process.execPath,
    [
      fileURLToPath(new URL("./publish-updater-manifest.mjs", import.meta.url)),
      "--verify-public",
    ],
    { env, encoding: "utf8" },
  );
  return { status: result.status, stderr: result.stderr };
}

assert.equal(
  runVerifyPublic("1.0.3", { version: "1.0.3", platforms: fromRelease }).status,
  0,
);
assert.notEqual(
  runVerifyPublic(undefined, { version: "1.0.3", platforms: fromRelease })
    .status,
  0,
);
assert.notEqual(
  runVerifyPublic("1.0.3", {
    version: "1.0.3",
    platforms: { ...fromRelease, "linux-x86_64-appimage": intel },
  }).status,
  0,
);
assert.notEqual(
  runVerifyPublic("1.0.3", {
    version: "1.0.3",
    platforms: {
      "darwin-x86_64": intel,
      "darwin-x86_64-app": intel,
    },
  }).status,
  0,
);

console.log("publish-updater-manifest: all assertions passed");
