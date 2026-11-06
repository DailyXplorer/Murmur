/**
 * @fileoverview Key rules of the unified updater manifest: asset mapping,
 * the public checks, the legacy Windows feed, and the proof that a Mac never
 * resolves to a Windows asset (and a PC never to a Mac one).
 */
import assert from "node:assert/strict";
import {
  assertLegacyWindowsFeed,
  assertPublicManifest,
  buildLegacyWindowsFeed,
  finalizePlatforms,
  resolveLikeUpdater,
  targetKey,
  updaterTarget,
} from "./updater-targets.mjs";

const base =
  "https://github.com/DailyXplorer/Murmur/releases/download/v1.13.0/";
const entry = (file, tag = "x") => ({
  signature: `signature-${tag}`,
  url: `${base}${file}`,
});
const arm = entry("Murmur_aarch64.app.tar.gz", "arm");
const intel = entry("Murmur_x64.app.tar.gz", "intel");
const exe = entry("Murmur_1.13.0_x64-setup.exe", "exe");

assert.equal(
  targetKey(updaterTarget("Murmur_1.13.0_x64-setup.exe.sig")),
  "windows-x86_64-nsis",
);
assert.equal(
  targetKey(updaterTarget("Murmur_1.13.0-test.2_x64-setup.exe.SIG")),
  "windows-x86_64-nsis",
);
assert.equal(
  targetKey(updaterTarget("Murmur_aarch64.app.tar.gz.sig")),
  "darwin-aarch64-app",
);
for (const ignored of [
  "Murmur_1.13.0_arm64-setup.exe.sig",
  "Murmur_1.13.0_x64-setup.exe",
  "Murmur_1.13.0_x64_en-US.msi.sig",
  "Murmur_x64-setup.exe.sig",
  "Other_1.13.0_x64-setup.exe.sig",
  "Murmur_1.13.0_x64.dmg",
  "Murmur_1.13.0_amd64.deb.sig",
  "Murmur_1.13.0_aarch64.AppImage.sig",
]) {
  assert.equal(updaterTarget(ignored), null, ignored);
}

const full = () =>
  finalizePlatforms(
    {
      "darwin-aarch64-app": arm,
      "darwin-x86_64-app": intel,
      "windows-x86_64-nsis": exe,
    },
    { windows: true },
  );
const platforms = full();
assert.equal(platforms["windows-x86_64"], exe);
assert.equal(platforms["darwin-aarch64"], arm);
assert.deepEqual(Object.keys(platforms).sort(), [
  "darwin-aarch64",
  "darwin-aarch64-app",
  "darwin-x86_64",
  "darwin-x86_64-app",
  "windows-x86_64",
  "windows-x86_64-nsis",
]);
assert.throws(
  () =>
    finalizePlatforms(
      { "darwin-aarch64-app": arm, "darwin-x86_64-app": intel },
      { windows: true },
    ),
  /windows-x86_64/,
);
assert.throws(
  () =>
    finalizePlatforms(
      {
        "darwin-aarch64-app": arm,
        "darwin-x86_64-app": intel,
        "windows-x86_64-nsis": exe,
      },
      { windows: false },
    ),
  /Unexpected updater platforms: windows-x86_64-nsis/,
);
assert.throws(
  () =>
    finalizePlatforms(
      {
        "darwin-aarch64-app": arm,
        "darwin-x86_64-app": intel,
        "linux-x86_64-appimage": intel,
      },
      { windows: false },
    ),
  /linux-x86_64-appimage/,
);

const unified = (overrides = {}) => ({
  version: "1.13.0",
  notes: "Notes",
  pub_date: "2026-11-06T12:00:00.000Z",
  platforms: { ...full(), ...overrides },
});
const macOnly = () => {
  const manifest = unified();
  delete manifest.platforms["windows-x86_64"];
  delete manifest.platforms["windows-x86_64-nsis"];
  return manifest;
};
const rejects = (manifest, pattern, options = { windows: true }) =>
  assert.throws(
    () => assertPublicManifest(manifest, "1.13.0", options),
    pattern,
  );

assertPublicManifest(unified(), "1.13.0", { windows: true });
assertPublicManifest(unified(), "1.13.0", {
  windows: true,
  repository: "DailyXplorer/Murmur",
  tag: "v1.13.0",
});
assertPublicManifest(macOnly(), "1.13.0", { windows: false });
assertPublicManifest(macOnly(), "1.13.0");

assert.throws(
  () => assertPublicManifest(unified(), "1.13.1", { windows: true }),
  /version mismatch/,
);
rejects(macOnly(), /Missing updater platform windows-x86_64/);
rejects(
  unified({ "windows-x86_64-nsis": { url: exe.url } }),
  /Missing updater platform windows-x86_64-nsis/,
);
rejects(unified(), /Non-darwin updater platforms present: windows-/, {
  windows: false,
});
rejects(
  unified({ "linux-x86_64-appimage": intel }),
  /Unexpected updater platforms present: linux-x86_64-appimage/,
);
rejects(
  unified({ "darwin-aarch64": entry("Murmur_1.13.0_x64-setup.exe") }),
  /darwin-aarch64 must point at a \.app\.tar\.gz/,
);
rejects(
  unified({ "darwin-aarch64-app": entry("Murmur_1.13.0_x64-setup.exe") }),
  /darwin-aarch64-app must point at a \.app\.tar\.gz/,
);
rejects(
  unified({ "windows-x86_64": entry("Murmur_aarch64.app.tar.gz") }),
  /windows-x86_64 must point at a -setup\.exe/,
);
rejects(
  unified({
    "windows-x86_64": {
      signature: "s",
      url: "https://example.com/a-setup.exe",
    },
  }),
  /not a GitHub release asset/,
);
rejects(
  unified({
    "windows-x86_64": {
      signature: "s",
      url: "https://github.com/DailyXplorer/Murmur/releases/download/v1.12.1/Murmur_1.12.1_x64-setup.exe",
    },
  }),
  /points at another release/,
);
assert.throws(
  () =>
    assertPublicManifest(unified(), "1.13.0", {
      windows: true,
      repository: "Someone/Else",
      tag: "v1.13.0",
    }),
  /must start with https:\/\/github\.com\/Someone\/Else\//,
);
assert.throws(
  () =>
    assertPublicManifest(unified(), "1.13.0", {
      windows: true,
      repository: "DailyXplorer/Murmur",
      tag: "v1.13.0-test.1",
    }),
  /must start with/,
);

const legacy = buildLegacyWindowsFeed(unified());
assert.deepEqual(legacy, {
  version: "1.13.0",
  notes: "Notes",
  pub_date: "2026-11-06T12:00:00.000Z",
  platforms: { "windows-x86_64": exe, "windows-x86_64-nsis": exe },
});
assert.deepEqual(Object.keys(legacy), [
  "version",
  "notes",
  "pub_date",
  "platforms",
]);
assertLegacyWindowsFeed(legacy, "1.13.0");
assert.throws(() => buildLegacyWindowsFeed(macOnly()), /windows-x86_64/);
assert.throws(
  () => assertLegacyWindowsFeed(legacy, "1.13.1"),
  /version mismatch/,
);
assert.throws(
  () => assertLegacyWindowsFeed(unified(), "1.13.0"),
  /Non-Windows updater platforms present: darwin-/,
);
assert.throws(
  () =>
    assertLegacyWindowsFeed(
      { ...legacy, platforms: { "windows-x86_64": exe } },
      "1.13.0",
    ),
  /Missing updater platform windows-x86_64-nsis/,
);
assert.throws(
  () =>
    assertLegacyWindowsFeed(
      {
        ...legacy,
        platforms: { ...legacy.platforms, "windows-x86_64": arm },
      },
      "1.13.0",
    ),
  /must point at a -setup\.exe/,
);

// The updater plugin's own order: `<os>-<arch>-<bundle>`, then `<os>-<arch>`.
const oses = ["darwin", "windows", "linux"];
const arches = ["aarch64", "x86_64", "i686", "armv7"];
const bundles = [undefined, "app", "nsis", "msi", "appimage", "deb", "rpm"];
const kindOf = (url) =>
  url.endsWith(".app.tar.gz") ? "mac" : url.endsWith("-setup.exe") ? "pc" : "?";

for (const manifest of [unified(), macOnly()]) {
  const hasWindows = "windows-x86_64" in manifest.platforms;
  for (const os of oses) {
    for (const arch of arches) {
      for (const bundle of bundles) {
        const found = resolveLikeUpdater(manifest, os, arch, bundle);
        const label = `${os}-${arch}-${bundle}`;
        if (os === "darwin") {
          assert.ok(
            !found || kindOf(found.url) === "mac",
            `${label} must never resolve to a Windows asset`,
          );
        } else if (os === "windows") {
          assert.ok(
            !found || kindOf(found.url) === "pc",
            `${label} must never resolve to a Mac asset`,
          );
        } else {
          assert.equal(found, null, label);
        }
      }
    }
  }

  for (const bundle of [undefined, "app", "dmg"]) {
    const aarch64 = resolveLikeUpdater(manifest, "darwin", "aarch64", bundle);
    const x86 = resolveLikeUpdater(manifest, "darwin", "x86_64", bundle);
    assert.equal(aarch64.url, arm.url);
    assert.equal(x86.url, intel.url);
  }
  assert.equal(
    resolveLikeUpdater(manifest, "darwin", "aarch64", "app").key,
    "darwin-aarch64-app",
  );
  assert.equal(resolveLikeUpdater(manifest, "darwin", "i686", "app"), null);

  const pc = resolveLikeUpdater(manifest, "windows", "x86_64", "nsis");
  assert.equal(
    pc === null,
    !hasWindows,
    "a Mac-only manifest offers no Windows asset",
  );
  if (pc) {
    assert.equal(pc.url, exe.url);
    assert.equal(pc.key, "windows-x86_64-nsis");
    assert.equal(
      resolveLikeUpdater(manifest, "windows", "x86_64", "msi").url,
      exe.url,
    );
    assert.equal(
      resolveLikeUpdater(manifest, "windows", "aarch64", "nsis"),
      null,
    );
  }
}

// Every key of the unified manifest is reachable only by its own family.
for (const key of Object.keys(unified().platforms)) {
  const [os, arch, bundle] = key.split("-");
  const found = resolveLikeUpdater(unified(), os, arch, bundle);
  assert.equal(found.key, key);
  assert.equal(kindOf(found.url), os === "darwin" ? "mac" : "pc");
}

// The legacy feed serves Windows only.
for (const arch of ["aarch64", "x86_64"]) {
  assert.equal(resolveLikeUpdater(legacy, "darwin", arch, "app"), null);
  assert.equal(resolveLikeUpdater(legacy, "darwin", arch), null);
}
assert.equal(
  resolveLikeUpdater(legacy, "windows", "x86_64", "nsis").url,
  exe.url,
);

// An entry without a signature does not count, as in the plugin.
assert.equal(
  resolveLikeUpdater(
    { platforms: { "darwin-aarch64-app": { url: arm.url } } },
    "darwin",
    "aarch64",
    "app",
  ),
  null,
);

console.log("updater-targets: all assertions passed");
