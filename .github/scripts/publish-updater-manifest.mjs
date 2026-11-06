import { readFile, writeFile } from "node:fs/promises";
import { pathToFileURL } from "node:url";
import {
  assertLegacyWindowsFeed,
  assertPublicManifest,
  buildLegacyWindowsFeed,
  finalizePlatforms,
  targetKey,
  updaterTarget,
} from "./updater-targets.mjs";
import { verifyManifestSignatures } from "./updater-signature.mjs";

export {
  DARWIN_ALIAS_ARCHES,
  REQUIRED_TARGETS,
  assertLegacyWindowsFeed,
  assertPublicManifest,
  buildLegacyWindowsFeed,
  finalizeDarwinPlatforms,
  finalizePlatforms,
  resolveLikeUpdater,
  updaterTarget,
} from "./updater-targets.mjs";
export { verifyManifestSignatures } from "./updater-signature.mjs";

/**
 * Asserts a public updater manifest is darwin-only and has signed ARM and Intel entries.
 * Kept for macOS-only callers; `assertPublicManifest` is the general check.
 * @param {Parameters<typeof assertPublicManifest>[0]} manifest
 * @param {string} version Expected `latest.json` version.
 * @returns {void}
 */
export function assertPublicDarwinManifest(manifest, version) {
  assertPublicManifest(manifest, version, { windows: false });
}

/**
 * Reads `INCLUDE_WINDOWS`: whether the release carries a Windows installer.
 * @param {Record<string, string | undefined>} env
 * @returns {boolean} `true` when unset.
 */
export function includeWindows(env) {
  const raw = env.INCLUDE_WINDOWS;
  if (raw === undefined || raw === "") return true;
  if (raw === "true") return true;
  if (raw === "false") return false;
  throw new Error(`INCLUDE_WINDOWS must be true or false, not ${raw}`);
}

/**
 * Authenticated GitHub REST helper.
 * @param {string} path API path beginning with `/`.
 * @param {RequestInit} [options]
 * @param {string} token GitHub token.
 * @returns {Promise<Response>}
 */
async function github(path, options = {}, token) {
  const headers = {
    Accept: "application/vnd.github+json",
    Authorization: `Bearer ${token}`,
    "X-GitHub-Api-Version": "2022-11-28",
    ...options.headers,
  };
  const response = await fetch(`https://api.github.com${path}`, {
    ...options,
    headers,
  });
  if (!response.ok) {
    throw new Error(
      `GitHub API ${options.method ?? "GET"} ${path} failed: ${response.status} ${await response.text()}`,
    );
  }
  return response;
}

/**
 * Lists all assets on a GitHub release, following pagination.
 * @param {string} owner
 * @param {string} repo
 * @param {number} releaseId
 * @param {string} token
 * @returns {Promise<Array<{id: number, name: string}>>}
 */
async function listAssets(owner, repo, releaseId, token) {
  const assets = [];
  for (let page = 1; ; page += 1) {
    const response = await github(
      `/repos/${owner}/${repo}/releases/${releaseId}/assets?per_page=100&page=${page}`,
      {},
      token,
    );
    const batch = await response.json();
    assets.push(...batch);
    if (batch.length < 100) return assets;
  }
}

/**
 * Downloads the minisign payload for a `.sig` release asset.
 * @param {{id: number}} asset
 * @param {string} owner
 * @param {string} repo
 * @param {string} token
 * @returns {Promise<string>}
 */
async function signatureFor(asset, owner, repo, token) {
  const response = await github(
    `/repos/${owner}/${repo}/releases/assets/${asset.id}`,
    { headers: { Accept: "application/octet-stream" } },
    token,
  );
  return (await response.text()).trim();
}

/**
 * Builds `latest.json` from the `.app.tar.gz.sig` assets (and, with `INCLUDE_WINDOWS`,
 * the NSIS `.exe.sig`) and uploads it to the release.
 * @returns {Promise<void>}
 */
async function publishUpdaterManifest() {
  const token = process.env.GITHUB_TOKEN;
  const repository = process.env.GITHUB_REPOSITORY;
  const releaseId = Number(process.env.RELEASE_ID);
  const version = process.env.VERSION;
  const windows = includeWindows(process.env);

  if (!token || !repository || !Number.isInteger(releaseId) || !version) {
    throw new Error(
      "GITHUB_TOKEN, GITHUB_REPOSITORY, RELEASE_ID, and VERSION are required",
    );
  }

  const [owner, repo] = repository.split("/");
  const releaseResponse = await github(
    `/repos/${owner}/${repo}/releases/${releaseId}`,
    {},
    token,
  );
  const release = await releaseResponse.json();
  const assets = await listAssets(owner, repo, releaseId, token);
  const assetsByName = new Map(assets.map((asset) => [asset.name, asset]));
  const platforms = {};

  for (const signatureAsset of assets.filter((asset) =>
    asset.name.endsWith(".sig"),
  )) {
    const target = updaterTarget(signatureAsset.name);
    if (!target || (target.os === "windows" && !windows)) continue;

    const installerName = signatureAsset.name.slice(0, -4);
    if (!assetsByName.has(installerName)) {
      throw new Error(`Missing installer for ${signatureAsset.name}`);
    }

    const signature = await signatureFor(signatureAsset, owner, repo, token);
    if (!signature)
      throw new Error(`Empty signature in ${signatureAsset.name}`);

    const encodedTag = encodeURIComponent(release.tag_name);
    const encodedName = encodeURIComponent(installerName);
    const entry = {
      signature,
      url: `https://github.com/${repository}/releases/download/${encodedTag}/${encodedName}`,
    };
    platforms[targetKey(target)] = entry;
  }

  finalizePlatforms(platforms, { windows });

  const manifest = {
    version,
    notes: release.body ?? "",
    pub_date: new Date().toISOString(),
    platforms: Object.fromEntries(
      Object.entries(platforms).sort(([left], [right]) =>
        left.localeCompare(right),
      ),
    ),
  };
  assertPublicManifest(manifest, version, {
    windows,
    repository,
    tag: release.tag_name,
  });
  const manifestJson = `${JSON.stringify(manifest, null, 2)}\n`;
  await writeFile("latest.json", manifestJson);

  const previousManifest = assetsByName.get("latest.json");
  if (previousManifest) {
    await github(
      `/repos/${owner}/${repo}/releases/assets/${previousManifest.id}`,
      { method: "DELETE" },
      token,
    );
  }

  const uploadUrl = new URL(
    `https://uploads.github.com/repos/${owner}/${repo}/releases/${releaseId}/assets`,
  );
  uploadUrl.searchParams.set("name", "latest.json");
  const uploadResponse = await fetch(uploadUrl, {
    method: "POST",
    headers: {
      Accept: "application/vnd.github+json",
      Authorization: `Bearer ${token}`,
      "X-GitHub-Api-Version": "2022-11-28",
      "Content-Type": "application/json",
      "Content-Length": String(Buffer.byteLength(manifestJson)),
    },
    body: manifestJson,
  });
  if (!uploadResponse.ok) {
    throw new Error(
      `Uploading latest.json failed: ${uploadResponse.status} ${await uploadResponse.text()}`,
    );
  }

  console.log(
    `Uploaded latest.json for ${version} with ${Object.keys(platforms).length} platform entries`,
  );
}

const isMain =
  Boolean(process.argv[1]) &&
  import.meta.url === pathToFileURL(process.argv[1]).href;

/**
 * @param {string} flag
 * @returns {string | undefined} The argument after `flag`, if the flag is present.
 */
function flagValue(flag) {
  const index = process.argv.indexOf(flag);
  if (index === -1) return undefined;
  const value = process.argv[index + 1];
  if (!value || value.startsWith("--"))
    throw new Error(`${flag} needs a value`);
  return value;
}

/**
 * @returns {string} The `VERSION` env.
 */
function requireVersion() {
  const version = process.env.VERSION;
  if (!version) throw new Error("VERSION is required");
  return version;
}

/**
 * @returns {{repository?: string, tag?: string}} Download prefix pins from the environment.
 */
function releasePins() {
  return {
    repository: process.env.GITHUB_REPOSITORY || undefined,
    tag: process.env.RELEASE_TAG || undefined,
  };
}

/**
 * Parses the `MANIFEST` env, the body of a freshly downloaded feed.
 * @returns {any}
 */
function manifestFromEnv() {
  const raw = process.env.MANIFEST;
  if (!raw) throw new Error("VERSION and MANIFEST are required");
  return JSON.parse(raw);
}

/**
 * CLI `--verify-public` path: assert VERSION and MANIFEST form the public `latest.json`.
 * @returns {void}
 */
function verifyPublicManifest() {
  const version = requireVersion();
  assertPublicManifest(manifestFromEnv(), version, {
    windows: includeWindows(process.env),
    ...releasePins(),
  });
}

/**
 * CLI `--verify-public-legacy` path: assert VERSION and MANIFEST form the Windows-only legacy feed.
 * @returns {void}
 */
function verifyPublicLegacyFeed() {
  assertLegacyWindowsFeed(manifestFromEnv(), requireVersion());
}

/**
 * Reads the unified manifest from `--manifest` (default `latest.json`).
 * @returns {Promise<any>}
 */
async function manifestFromFile() {
  return JSON.parse(
    await readFile(flagValue("--manifest") ?? "latest.json", "utf8"),
  );
}

/**
 * CLI `--emit-legacy-feed <path>`: write the Windows-only feed of the unified manifest.
 * @param {string} outPath
 * @returns {Promise<void>}
 */
async function emitLegacyFeed(outPath) {
  const version = requireVersion();
  const manifest = await manifestFromFile();
  assertPublicManifest(manifest, version, { windows: true, ...releasePins() });
  const feed = buildLegacyWindowsFeed(manifest);
  assertLegacyWindowsFeed(feed, version);
  await writeFile(outPath, `${JSON.stringify(feed, null, 2)}\n`);
}

/**
 * CLI `--verify-signatures <dir>`: check every manifest entry against the downloaded
 * asset in `<dir>` with the updater `pubkey` of `--config` (default `src-tauri/tauri.conf.json`).
 * @param {string} directory
 * @returns {Promise<void>}
 */
async function verifySignatures(directory) {
  const version = requireVersion();
  const manifest = await manifestFromFile();
  assertPublicManifest(manifest, version, {
    windows: includeWindows(process.env),
    ...releasePins(),
  });
  const config = JSON.parse(
    await readFile(
      flagValue("--config") ?? "src-tauri/tauri.conf.json",
      "utf8",
    ),
  );
  const pubkey = config?.plugins?.updater?.pubkey;
  if (!pubkey) throw new Error("No updater pubkey in the Tauri config");
  const keys = await verifyManifestSignatures(manifest, {
    directory,
    pubkey,
    version,
  });
  console.log(`Verified ${keys.length} updater signatures for ${version}`);
}

if (isMain) {
  if (process.argv.includes("--verify-public")) {
    verifyPublicManifest();
  } else if (process.argv.includes("--verify-public-legacy")) {
    verifyPublicLegacyFeed();
  } else if (process.argv.includes("--emit-legacy-feed")) {
    await emitLegacyFeed(flagValue("--emit-legacy-feed"));
  } else if (process.argv.includes("--verify-signatures")) {
    await verifySignatures(flagValue("--verify-signatures"));
  } else {
    await publishUpdaterManifest();
  }
}
