import { writeFile } from "node:fs/promises";
import { pathToFileURL } from "node:url";

/**
 * Canonical darwin updater platform keys that must exist in `latest.json`.
 * @type {readonly string[]}
 */
export const REQUIRED_TARGETS = ["darwin-aarch64-app", "darwin-x86_64-app"];

/**
 * Darwin architectures that also receive alias keys without the `-app` suffix.
 * @type {readonly string[]}
 */
export const DARWIN_ALIAS_ARCHES = ["aarch64", "x86_64"];

/**
 * Maps a GitHub release asset name to a darwin updater target, or `null` if ignored.
 * @param {string} assetName Release asset filename.
 * @returns {{os: "darwin", arch: string, bundle: "app"} | null}
 */
export function updaterTarget(assetName) {
  const name = assetName.toLowerCase();
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
 * Copies `darwin-{arch}-app` entries onto alias keys and asserts required targets.
 * @param {Record<string, {url: string, signature: string}>} platforms
 * @returns {Record<string, {url: string, signature: string}>}
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
 * Asserts a public updater manifest is darwin-only and has signed ARM and Intel entries.
 * @param {{version?: string, platforms?: Record<string, {url?: string, signature?: string}>}} manifest
 * @param {string} version Expected `latest.json` version.
 * @returns {void}
 */
export function assertPublicDarwinManifest(manifest, version) {
  if (!manifest || manifest.version !== version) {
    throw new Error("Updater manifest version mismatch");
  }

  const platforms = manifest.platforms ?? {};
  for (const key of [
    "darwin-aarch64",
    "darwin-aarch64-app",
    "darwin-x86_64",
    "darwin-x86_64-app",
  ]) {
    const entry = platforms[key];
    if (!entry?.url || !entry?.signature) {
      throw new Error(`Missing updater platform ${key}`);
    }
  }

  const extra = Object.keys(platforms).filter(
    (key) => !key.startsWith("darwin-"),
  );
  if (extra.length > 0) {
    throw new Error(
      `Non-darwin updater platforms present: ${extra.join(", ")}`,
    );
  }
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
 * Builds `latest.json` from darwin `.app.tar.gz.sig` assets and uploads it to the release.
 * @returns {Promise<void>}
 */
async function publishUpdaterManifest() {
  const token = process.env.GITHUB_TOKEN;
  const repository = process.env.GITHUB_REPOSITORY;
  const releaseId = Number(process.env.RELEASE_ID);
  const version = process.env.VERSION;

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
    if (!target) continue;

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
    platforms[`${target.os}-${target.arch}-${target.bundle}`] = entry;
  }

  finalizeDarwinPlatforms(platforms);

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
  assertPublicDarwinManifest(manifest, version);
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
 * CLI `--verify-public` path: assert VERSION and MANIFEST form a darwin-only `latest.json`.
 * @returns {void}
 */
function verifyPublicManifest() {
  const version = process.env.VERSION;
  const raw = process.env.MANIFEST;
  if (!version || !raw) {
    throw new Error("VERSION and MANIFEST are required");
  }
  assertPublicDarwinManifest(JSON.parse(raw), version);
}

if (isMain) {
  if (process.argv.includes("--verify-public")) {
    verifyPublicManifest();
  } else {
    await publishUpdaterManifest();
  }
}
