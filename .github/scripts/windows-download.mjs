import { createHash } from "node:crypto";
import { pathToFileURL } from "node:url";

/**
 * Moving pre-release that holds the current Windows installer under a fixed name.
 * It is never the latest release, so the macOS updater keeps reading `latest.json`
 * from the newest full release. Mac and Windows share a release in issue 157.
 */
export const WINDOWS_INSTALLER_TAG = "windows-installer";

/** Asset name used by the README. It does not include a version. */
export const WINDOWS_INSTALLER_ASSET = "Murmur_x64-setup.exe";

/**
 * @param {string} repo `owner/name`
 * @returns {string}
 */
export function stableDownloadUrl(repo) {
  return `https://github.com/${repo}/releases/download/${WINDOWS_INSTALLER_TAG}/${WINDOWS_INSTALLER_ASSET}`;
}

/**
 * A versioned NSIS installer. The stable asset name is reserved for the pointer.
 * @param {string} name
 * @returns {boolean}
 */
export function isVersionedWindowsInstaller(name) {
  return /x64-setup\.exe$/i.test(name) && name !== WINDOWS_INSTALLER_ASSET;
}

/**
 * @param {string | null | undefined} digest
 * @returns {string | null}
 */
export function digestHex(digest) {
  if (!digest) return null;
  const hex = digest.toLowerCase().replace(/^sha256:/, "");
  return /^[0-9a-f]{64}$/.test(hex) ? hex : null;
}

/**
 * @param {{tag_name?: string, draft?: boolean, published_at?: string | null, target_commitish?: string, assets?: Array<{name: string, digest?: string | null, size?: number, url?: string}>}} release
 * @returns {{name: string, digest?: string | null, size?: number, url?: string} | null}
 */
function versionedInstaller(release) {
  const versioned = (release.assets ?? []).filter((asset) =>
    isVersionedWindowsInstaller(asset.name),
  );
  if (versioned.length > 1) {
    throw new Error(
      `Release ${release.tag_name} has more than one Windows installer: ${versioned
        .map((asset) => asset.name)
        .join(", ")}`,
    );
  }
  return versioned[0] ?? null;
}

/**
 * The newest published release that contains a versioned Windows installer.
 * The pointer release is ignored so it cannot select itself.
 * @param {Array<{tag_name?: string, draft?: boolean, published_at?: string | null, target_commitish?: string, assets?: Array<{name: string, digest?: string | null, size?: number, url?: string}>}>} releases
 * @returns {{tagName: string, assetName: string, digest: string | null, size: number | null, publishedAt: string | null, targetCommitish: string | null, assetUrl: string | null} | null}
 */
export function newestWindowsRelease(releases) {
  const candidates = [];
  for (const release of releases) {
    if (release.draft || release.tag_name === WINDOWS_INSTALLER_TAG) continue;
    const asset = versionedInstaller(release);
    if (!asset) continue;
    candidates.push({ release, asset });
  }

  candidates.sort((left, right) => {
    const byDate = publishedAt(right.release) - publishedAt(left.release);
    if (byDate !== 0) return byDate;
    return (right.release.tag_name ?? "").localeCompare(
      left.release.tag_name ?? "",
    );
  });

  const chosen = candidates[0];
  if (!chosen) return null;
  return {
    tagName: chosen.release.tag_name ?? "",
    assetName: chosen.asset.name,
    digest: digestHex(chosen.asset.digest),
    size: Number.isFinite(chosen.asset.size) ? chosen.asset.size : null,
    publishedAt: chosen.release.published_at ?? null,
    targetCommitish: chosen.release.target_commitish ?? null,
    assetUrl: chosen.asset.url ?? null,
  };
}

/**
 * @param {{published_at?: string | null}} release
 * @returns {number}
 */
function publishedAt(release) {
  const parsed = Date.parse(release.published_at ?? "");
  return Number.isNaN(parsed) ? 0 : parsed;
}

/**
 * Notes for the pointer release. Avoids GitHub closing keywords.
 * @param {{repo: string, tagName: string, assetName: string}} source
 * @returns {string}
 */
export function releaseNotes(source) {
  return [
    "Current Windows installer for the README download link.",
    "",
    `Copied from [\`${source.tagName}\`](https://github.com/${source.repo}/releases/tag/${encodeURIComponent(source.tagName)}) (\`${source.assetName}\`).`,
    "",
    "This pre-release is not a macOS build and is not the latest release. macOS updates keep using `latest.json` on the latest full release. Mac and Windows ship as one release in https://github.com/DailyXplorer/Murmur/issues/157.",
    "",
  ].join("\n");
}

/**
 * @param {string} path API path beginning with `/`.
 * @param {RequestInit} [options]
 * @param {string} token
 * @returns {Promise<Response>}
 */
async function github(path, options = {}, token) {
  const headers = {
    Accept: "application/vnd.github+json",
    Authorization: `Bearer ${token}`,
    "X-GitHub-Api-Version": "2022-11-28",
    "User-Agent": "murmur-windows-download",
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
 * @param {string} owner
 * @param {string} repo
 * @param {string} token
 * @returns {Promise<Array<object>>}
 */
async function listReleases(owner, repo, token) {
  const releases = [];
  for (let page = 1; ; page += 1) {
    const response = await github(
      `/repos/${owner}/${repo}/releases?per_page=100&page=${page}`,
      {},
      token,
    );
    const batch = await response.json();
    releases.push(...batch);
    if (batch.length < 100) return releases;
  }
}

/**
 * @param {string} owner
 * @param {string} repo
 * @param {string} tag
 * @param {string} token
 * @returns {Promise<string>}
 */
async function commitForTag(owner, repo, tag, token) {
  const response = await github(
    `/repos/${owner}/${repo}/git/ref/tags/${encodeURIComponent(tag)}`,
    {},
    token,
  );
  const ref = await response.json();
  if (ref.object?.type === "commit") return ref.object.sha;
  if (ref.object?.type === "tag") {
    const annotated = await github(
      `/repos/${owner}/${repo}/git/tags/${ref.object.sha}`,
      {},
      token,
    );
    const tagObject = await annotated.json();
    if (tagObject.object?.type === "commit") return tagObject.object.sha;
  }
  throw new Error(`Tag ${tag} does not point at a commit`);
}

/**
 * @param {string} owner
 * @param {string} repo
 * @param {string} token
 * @returns {Promise<object | null>}
 */
async function pointerRelease(owner, repo, token) {
  const response = await fetch(
    `https://api.github.com/repos/${owner}/${repo}/releases/tags/${WINDOWS_INSTALLER_TAG}`,
    {
      headers: {
        Accept: "application/vnd.github+json",
        Authorization: `Bearer ${token}`,
        "X-GitHub-Api-Version": "2022-11-28",
        "User-Agent": "murmur-windows-download",
      },
    },
  );
  if (response.status === 404) return null;
  if (!response.ok) {
    throw new Error(
      `GitHub API GET pointer release failed: ${response.status} ${await response.text()}`,
    );
  }
  return response.json();
}

/**
 * @param {{url?: string | null, size?: number | null, digest?: string | null, assetName?: string}} source
 * @param {string} token
 * @returns {Promise<Buffer>}
 */
async function downloadInstaller(source, token) {
  if (!source.assetUrl) throw new Error("Windows installer has no asset URL");
  const response = await github(
    new URL(source.assetUrl).pathname,
    { headers: { Accept: "application/octet-stream" } },
    token,
  );
  const bytes = Buffer.from(await response.arrayBuffer());
  if (bytes.length < 2 || bytes[0] !== 0x4d || bytes[1] !== 0x5a) {
    throw new Error(
      `Downloaded ${source.assetName} is not a Windows executable`,
    );
  }
  if (source.size != null && bytes.length !== source.size) {
    throw new Error(
      `Downloaded ${source.assetName} is ${bytes.length} bytes, expected ${source.size}`,
    );
  }
  const hash = createHash("sha256").update(bytes).digest("hex");
  if (source.digest && hash !== source.digest) {
    throw new Error(`Downloaded ${source.assetName} checksum does not match`);
  }
  return bytes;
}

/**
 * @param {string} owner
 * @param {string} repo
 * @param {number} releaseId
 * @param {Buffer} bytes
 * @param {string} token
 * @returns {Promise<void>}
 */
async function uploadInstaller(owner, repo, releaseId, bytes, token) {
  const uploadUrl = new URL(
    `https://uploads.github.com/repos/${owner}/${repo}/releases/${releaseId}/assets`,
  );
  uploadUrl.searchParams.set("name", WINDOWS_INSTALLER_ASSET);
  const response = await fetch(uploadUrl, {
    method: "POST",
    headers: {
      Accept: "application/vnd.github+json",
      Authorization: `Bearer ${token}`,
      "X-GitHub-Api-Version": "2022-11-28",
      "Content-Type": "application/octet-stream",
      "Content-Length": String(bytes.length),
      "User-Agent": "murmur-windows-download",
    },
    body: bytes,
  });
  if (!response.ok) {
    throw new Error(
      `Uploading ${WINDOWS_INSTALLER_ASSET} failed: ${response.status} ${await response.text()}`,
    );
  }
}

/**
 * Copies the newest versioned Windows installer onto the stable pre-release.
 * @returns {Promise<void>}
 */
export async function syncWindowsInstaller() {
  const token = process.env.GITHUB_TOKEN || process.env.GH_TOKEN;
  const repository = process.env.GITHUB_REPOSITORY || "DailyXplorer/Murmur";
  if (!token) throw new Error("GITHUB_TOKEN or GH_TOKEN is required");
  const [owner, repo] = repository.split("/");
  if (!owner || !repo) throw new Error(`Invalid repository ${repository}`);

  const chosen = newestWindowsRelease(await listReleases(owner, repo, token));
  if (!chosen) throw new Error("No published Windows installer was found");
  if (!chosen.assetUrl) {
    throw new Error(`Missing asset URL for ${chosen.assetName}`);
  }

  const notes = releaseNotes({
    repo: repository,
    tagName: chosen.tagName,
    assetName: chosen.assetName,
  });
  const existing = await pointerRelease(owner, repo, token);
  const existingAsset = existing?.assets?.find(
    (asset) => asset.name === WINDOWS_INSTALLER_ASSET,
  );
  if (
    existing &&
    chosen.digest &&
    digestHex(existingAsset?.digest) === chosen.digest
  ) {
    console.log(
      `Windows installer already matches ${chosen.tagName} (${chosen.assetName})`,
    );
    return;
  }

  const sha = await commitForTag(owner, repo, chosen.tagName, token);
  const bytes = await downloadInstaller(chosen, token);
  let releaseId = existing?.id;
  if (!existing) {
    const created = await github(
      `/repos/${owner}/${repo}/releases`,
      {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({
          tag_name: WINDOWS_INSTALLER_TAG,
          target_commitish: sha,
          name: "Windows installer",
          body: notes,
          draft: false,
          prerelease: true,
          make_latest: "false",
        }),
      },
      token,
    );
    releaseId = (await created.json()).id;
  } else if (existingAsset?.id) {
    await github(
      `/repos/${owner}/${repo}/releases/assets/${existingAsset.id}`,
      { method: "DELETE" },
      token,
    );
  }

  await uploadInstaller(owner, repo, releaseId, bytes, token);
  if (existing) {
    await github(
      `/repos/${owner}/${repo}/releases/${releaseId}`,
      {
        method: "PATCH",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({
          name: "Windows installer",
          body: notes,
          prerelease: true,
          make_latest: "false",
          target_commitish: sha,
        }),
      },
      token,
    );
  }
  console.log(
    `Windows installer updated from ${chosen.tagName} (${bytes.length} bytes)`,
  );
}

const isMain =
  Boolean(process.argv[1]) &&
  import.meta.url === pathToFileURL(process.argv[1]).href;

if (isMain) {
  if (!process.argv.includes("--sync")) {
    throw new Error("Pass --sync to refresh the stable Windows installer");
  }
  await syncWindowsInstaller();
}
