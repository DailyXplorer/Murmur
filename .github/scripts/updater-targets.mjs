/**
 * @fileoverview Which updater keys a Murmur release may carry, which asset
 * each one points at, and how the Tauri updater plugin looks a key up.
 *
 * A macOS client only ever reads `darwin-*` keys and must get a
 * `.app.tar.gz`; a Windows client only reads `windows-*` keys and must get
 * the NSIS `-setup.exe`. Nothing else may sit in `latest.json`.
 */

/**
 * @typedef {{url?: string, signature?: string}} PlatformEntry
 * @typedef {{os: "darwin" | "windows", arch: string, bundle: "app" | "nsis"}} UpdaterTarget
 */

/**
 * Darwin architectures that also receive alias keys without the `-app` suffix.
 * @type {readonly string[]}
 */
export const DARWIN_ALIAS_ARCHES = ["aarch64", "x86_64"];

/** @type {readonly string[]} */
export const DARWIN_KEYS = [
  "darwin-aarch64",
  "darwin-aarch64-app",
  "darwin-x86_64",
  "darwin-x86_64-app",
];

/** @type {readonly string[]} */
export const WINDOWS_KEYS = ["windows-x86_64", "windows-x86_64-nsis"];

/** Canonical keys that must be built from a signed asset (aliases are copies). */
export const REQUIRED_TARGETS = ["darwin-aarch64-app", "darwin-x86_64-app"];

const DARWIN_URL_SUFFIX = ".app.tar.gz";
const WINDOWS_URL_SUFFIX = "-setup.exe";
const WINDOWS_SIG = /^murmur_[^_/]+_x64-setup\.exe\.sig$/;

/**
 * Maps a GitHub release asset name to an updater target, or `null` if ignored.
 * Only the `.app.tar.gz.sig` of a Mac build and the x64 NSIS `.exe.sig` map.
 * @param {string} assetName Release asset filename.
 * @returns {UpdaterTarget | null}
 */
export function updaterTarget(assetName) {
  const name = assetName.toLowerCase();
  if (WINDOWS_SIG.test(name)) {
    return { os: "windows", arch: "x86_64", bundle: "nsis" };
  }
  if (!name.endsWith(".app.tar.gz.sig")) {
    return null;
  }

  let arch;
  if (/(?:aarch64|arm64)/.test(name)) arch = "aarch64";
  else if (/(?:x86_64|amd64|x64)/.test(name)) arch = "x86_64";
  else return null;

  return { os: "darwin", arch, bundle: "app" };
}

/**
 * @param {UpdaterTarget} target
 * @returns {string} The canonical key, such as `darwin-aarch64-app`.
 */
export function targetKey(target) {
  return `${target.os}-${target.arch}-${target.bundle}`;
}

/**
 * The exact set of keys a release publishes.
 * @param {{windows: boolean}} options
 * @returns {string[]}
 */
export function expectedKeys({ windows }) {
  return windows ? [...DARWIN_KEYS, ...WINDOWS_KEYS] : [...DARWIN_KEYS];
}

/**
 * Copies `darwin-{arch}-app` entries onto alias keys and asserts required targets.
 * @param {Record<string, PlatformEntry>} platforms
 * @returns {Record<string, PlatformEntry>}
 */
export function finalizeDarwinPlatforms(platforms) {
  for (const arch of DARWIN_ALIAS_ARCHES) {
    const entry = platforms[`darwin-${arch}-app`];
    if (!entry) throw new Error(`Missing updater for darwin-${arch}`);
    platforms[`darwin-${arch}`] = entry;
  }

  for (const target of REQUIRED_TARGETS) {
    if (!platforms[target]) throw new Error(`Missing updater target ${target}`);
  }

  return platforms;
}

/**
 * Aliases Darwin (and, when asked, Windows) keys, then rejects a missing or
 * foreign key.
 * @param {Record<string, PlatformEntry>} platforms
 * @param {{windows: boolean}} options
 * @returns {Record<string, PlatformEntry>}
 */
export function finalizePlatforms(platforms, { windows }) {
  finalizeDarwinPlatforms(platforms);
  if (windows) {
    const entry = platforms["windows-x86_64-nsis"];
    if (!entry) throw new Error("Missing updater for windows-x86_64");
    platforms["windows-x86_64"] = entry;
  }

  const allowed = new Set(expectedKeys({ windows }));
  const foreign = Object.keys(platforms).filter((key) => !allowed.has(key));
  if (foreign.length > 0) {
    throw new Error(`Unexpected updater platforms: ${foreign.join(", ")}`);
  }
  return platforms;
}

/**
 * Throws unless `url` is the asset kind the client of `key` installs.
 * @param {string} key A `darwin-*` or `windows-*` key.
 * @param {string} url
 * @returns {void}
 */
export function assertAssetKind(key, url) {
  const suffix = key.startsWith("darwin-")
    ? DARWIN_URL_SUFFIX
    : WINDOWS_URL_SUFFIX;
  if (!url.endsWith(suffix)) {
    throw new Error(`Updater platform ${key} must point at a ${suffix} asset`);
  }
}

/**
 * Asserts a public updater manifest has the signed entries of this release,
 * the right asset kind behind every key, URLs from one release, and no other key.
 * @param {{version?: string, platforms?: Record<string, PlatformEntry>}} manifest
 * @param {string} version Expected `latest.json` version.
 * @param {{windows?: boolean, repository?: string, tag?: string}} [options]
 *   `repository` (`owner/repo`) and `tag` pin the download prefix when known.
 * @returns {void}
 */
export function assertPublicManifest(manifest, version, options = {}) {
  const windows = options.windows ?? false;
  if (!manifest || manifest.version !== version) {
    throw new Error("Updater manifest version mismatch");
  }

  const platforms = manifest.platforms ?? {};
  for (const key of expectedKeys({ windows })) {
    const entry = platforms[key];
    if (!entry?.url || !entry?.signature) {
      throw new Error(`Missing updater platform ${key}`);
    }
  }

  const foreign = Object.keys(platforms).filter(
    (key) => !expectedKeys({ windows }).includes(key),
  );
  if (foreign.length > 0) {
    const kind = windows ? "Unexpected" : "Non-darwin";
    throw new Error(`${kind} updater platforms present: ${foreign.join(", ")}`);
  }

  let release = null;
  for (const [key, entry] of Object.entries(platforms)) {
    assertAssetKind(key, entry.url);
    const match =
      /^(https:\/\/github\.com\/[^/]+\/[^/]+\/releases\/download\/[^/]+\/)[^/]+$/.exec(
        entry.url,
      );
    if (!match) {
      throw new Error(`Updater platform ${key} is not a GitHub release asset`);
    }
    release ??= match[1];
    if (match[1] !== release) {
      throw new Error(`Updater platform ${key} points at another release`);
    }
  }

  if (options.repository && options.tag) {
    const expected = `https://github.com/${options.repository}/releases/download/${encodeURIComponent(options.tag)}/`;
    if (release !== expected) {
      throw new Error(`Updater URLs must start with ${expected}`);
    }
  }
}

/**
 * Pure rule behind the legacy `windows-beta` feed: the unified release's
 * Windows entries, nothing else.
 * @param {{version: string, notes?: string, pub_date?: string, platforms: Record<string, PlatformEntry>}} manifest
 * @returns {{version: string, notes?: string, pub_date?: string, platforms: Record<string, PlatformEntry>}}
 */
export function buildLegacyWindowsFeed(manifest) {
  const platforms = {};
  for (const key of WINDOWS_KEYS) {
    const entry = manifest.platforms?.[key];
    if (!entry?.url || !entry?.signature) {
      throw new Error(`Missing updater platform ${key}`);
    }
    platforms[key] = { signature: entry.signature, url: entry.url };
  }
  return {
    version: manifest.version,
    notes: manifest.notes,
    pub_date: manifest.pub_date,
    platforms,
  };
}

/**
 * Asserts a feed is the Windows-only manifest of the legacy `windows-beta` pointer.
 * @param {{version?: string, platforms?: Record<string, PlatformEntry>}} feed
 * @param {string} version Expected feed version.
 * @returns {void}
 */
export function assertLegacyWindowsFeed(feed, version) {
  if (!feed || feed.version !== version) {
    throw new Error("Legacy Windows feed version mismatch");
  }
  const platforms = feed.platforms ?? {};
  const foreign = Object.keys(platforms).filter(
    (key) => !WINDOWS_KEYS.includes(key),
  );
  if (foreign.length > 0) {
    throw new Error(
      `Non-Windows updater platforms present: ${foreign.join(", ")}`,
    );
  }
  for (const key of WINDOWS_KEYS) {
    const entry = platforms[key];
    if (!entry?.url || !entry?.signature) {
      throw new Error(`Missing updater platform ${key}`);
    }
    assertAssetKind(key, entry.url);
  }
}

/**
 * Finds the entry the Tauri updater plugin (`get_urls` in tauri-plugin-updater
 * 2.x) picks: `<os>-<arch>-<bundle>` first, then `<os>-<arch>`; an entry
 * counts only with both `url` and `signature`.
 * @param {{platforms?: Record<string, PlatformEntry>}} manifest
 * @param {string} os `darwin`, `windows`, ...
 * @param {string} arch `aarch64`, `x86_64`, ...
 * @param {string} [bundle] The installed bundle type (`app`, `nsis`, `msi`, ...), if known.
 * @returns {{key: string, url: string, signature: string} | null} `null` when the plugin finds no target.
 */
export function resolveLikeUpdater(manifest, os, arch, bundle) {
  const keys = bundle ? [`${os}-${arch}-${bundle}`] : [];
  keys.push(`${os}-${arch}`);
  for (const key of keys) {
    const entry = manifest.platforms?.[key];
    if (entry?.url && entry?.signature) {
      return { key, url: entry.url, signature: entry.signature };
    }
  }
  return null;
}
