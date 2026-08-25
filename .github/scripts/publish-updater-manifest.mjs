import { writeFile } from "node:fs/promises";

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
const apiBase = "https://api.github.com";
const headers = {
  Accept: "application/vnd.github+json",
  Authorization: `Bearer ${token}`,
  "X-GitHub-Api-Version": "2022-11-28",
};

async function github(path, options = {}) {
  const response = await fetch(`${apiBase}${path}`, {
    ...options,
    headers: { ...headers, ...options.headers },
  });
  if (!response.ok) {
    throw new Error(
      `GitHub API ${options.method ?? "GET"} ${path} failed: ${response.status} ${await response.text()}`,
    );
  }
  return response;
}

async function listAssets() {
  const assets = [];
  for (let page = 1; ; page += 1) {
    const response = await github(
      `/repos/${owner}/${repo}/releases/${releaseId}/assets?per_page=100&page=${page}`,
    );
    const batch = await response.json();
    assets.push(...batch);
    if (batch.length < 100) return assets;
  }
}

function updaterTarget(assetName) {
  const name = assetName.toLowerCase();
  let os;
  let bundle;

  if (name.endsWith(".app.tar.gz.sig")) {
    os = "darwin";
    bundle = "app";
  } else if (name.endsWith(".appimage.sig")) {
    os = "linux";
    bundle = "appimage";
  } else if (name.endsWith(".deb.sig")) {
    os = "linux";
    bundle = "deb";
  } else if (name.endsWith(".rpm.sig")) {
    os = "linux";
    bundle = "rpm";
  } else if (name.endsWith(".msi.sig")) {
    os = "windows";
    bundle = "msi";
  } else if (name.endsWith(".exe.sig")) {
    os = "windows";
    bundle = "nsis";
  } else {
    return null;
  }

  let arch;
  if (/(?:aarch64|arm64)/.test(name)) arch = "aarch64";
  else if (/(?:x86_64|amd64|x64)/.test(name)) arch = "x86_64";
  else if (/(?:i686|i386|x86)/.test(name)) arch = "i686";
  else return null;

  return { os, arch, bundle };
}

async function signatureFor(asset) {
  const response = await github(
    `/repos/${owner}/${repo}/releases/assets/${asset.id}`,
    { headers: { Accept: "application/octet-stream" } },
  );
  return (await response.text()).trim();
}

const releaseResponse = await github(
  `/repos/${owner}/${repo}/releases/${releaseId}`,
);
const release = await releaseResponse.json();
const assets = await listAssets();
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

  const signature = await signatureFor(signatureAsset);
  if (!signature) throw new Error(`Empty signature in ${signatureAsset.name}`);

  const encodedTag = encodeURIComponent(release.tag_name);
  const encodedName = encodeURIComponent(installerName);
  const entry = {
    signature,
    url: `https://github.com/${repository}/releases/download/${encodedTag}/${encodedName}`,
  };
  platforms[`${target.os}-${target.arch}-${target.bundle}`] = entry;
}

for (const os of ["darwin", "linux", "windows"]) {
  for (const arch of ["aarch64", "x86_64"]) {
    const preferredBundles =
      os === "darwin"
        ? ["app"]
        : os === "linux"
          ? ["appimage", "deb", "rpm"]
          : ["nsis", "msi"];
    const entry = preferredBundles
      .map((bundle) => platforms[`${os}-${arch}-${bundle}`])
      .find(Boolean);
    if (!entry) throw new Error(`Missing updater for ${os}-${arch}`);
    platforms[`${os}-${arch}`] = entry;
  }
}

const requiredTargets = [
  "darwin-aarch64-app",
  "darwin-x86_64-app",
  "linux-aarch64-appimage",
  "linux-x86_64-appimage",
  "windows-aarch64-nsis",
  "windows-x86_64-nsis",
];
for (const target of requiredTargets) {
  if (!platforms[target]) throw new Error(`Missing updater target ${target}`);
}

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
const manifestJson = `${JSON.stringify(manifest, null, 2)}\n`;
await writeFile("latest.json", manifestJson);

const previousManifest = assetsByName.get("latest.json");
if (previousManifest) {
  await github(
    `/repos/${owner}/${repo}/releases/assets/${previousManifest.id}`,
    { method: "DELETE" },
  );
}

const uploadUrl = new URL(
  `https://uploads.github.com/repos/${owner}/${repo}/releases/${releaseId}/assets`,
);
uploadUrl.searchParams.set("name", "latest.json");
const uploadResponse = await fetch(uploadUrl, {
  method: "POST",
  headers: {
    ...headers,
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
