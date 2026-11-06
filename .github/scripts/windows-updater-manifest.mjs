/**
 * @fileoverview The Windows beta update feed. A pushed tag `vX.Y.Z-beta.N`
 * releases the NSIS installer and its signature; this script builds the
 * `latest.json` the Windows app reads from the rolling `windows-beta` release,
 * checks the signature offline, and checks the public result afterwards.
 *
 * The feed holds `windows-*` keys only. The macOS feed (the latest release)
 * is built by publish-updater-manifest.mjs and is only read here, to prove a
 * Windows beta never reached it.
 *
 *   notes        --version V --message F --installer F --out F
 *   build        --installer F --sig F --tag T --notes F --repository R --out F [--pub-date D]
 *   verify       --installer F --manifest F --tag T --config tauri.conf.json
 *   verify-public --tag T --repository R --config tauri.conf.json
 */
import { createHash } from "node:crypto";
import { readFileSync, writeFileSync } from "node:fs";
import path from "node:path";
import { pathToFileURL } from "node:url";
import { verifySignature } from "./minisign.mjs";

export const FEED_TAG = "windows-beta";
export const WINDOWS_TARGETS = ["windows-x86_64", "windows-x86_64-nsis"];
const TAG_PATTERN = /^v(\d+\.\d+\.\d+-beta\.\d+)$/;
const CHECKSUM_LINE = /^SHA-256: ([0-9a-f]{64})\s*$/m;

/**
 * @param {string} tag
 * @returns {string} The version the tag names, without the `v`.
 */
export function versionOfTag(tag) {
  const match = TAG_PATTERN.exec(tag);
  if (!match) throw new Error(`tag ${tag} is not vX.Y.Z-beta.N`);
  return match[1];
}

/** @param {string} version */
export function installerName(version) {
  return `Murmur_${version}_x64-setup.exe`;
}

/**
 * @param {string} repository `owner/repo`
 * @param {string} tag
 * @param {string} name
 */
export function assetUrl(repository, tag, name) {
  return `https://github.com/${repository}/releases/download/${encodeURIComponent(tag)}/${encodeURIComponent(name)}`;
}

/**
 * The release notes: the tag message inside a fixed layout, ending with the
 * installer's checksum on a line of its own (verify-public reads it back).
 * @param {{version: string, message: string, checksum: string}} input
 * @returns {string}
 */
export function releaseNotes({ version, message, checksum }) {
  const name = installerName(version);
  const sections = [
    `## Murmur for Windows ${version}`,
    message.trim(),
    "Not on Windows yet: meetings, call features.",
    [
      "### Install without administrator rights",
      `1. Download \`${name}\` below and run it.`,
      '2. The installer has no publisher certificate: if SmartScreen shows "Windows protected your PC", select **More info**, then **Run anyway**.',
      "3. Murmur installs for your account only, in `%LOCALAPPDATA%\\Murmur`, and appears in the Start menu. A new beta installs over the previous one.",
    ].join("\n"),
    [
      "### Updates",
      "Beta 7 is the first beta that updates itself. If you run beta 6 or an older beta, install this build by hand once. After that Murmur offers each new beta: the footer says **Update available**, and **Check for updates** in the footer or the tray menu looks right away. Settings, Advanced, Updates turns the automatic checks off.",
    ].join("\n"),
    `SHA-256: ${checksum}`,
  ];
  return `${sections.filter(Boolean).join("\n\n")}\n`;
}

/**
 * @param {{tag: string, notes: string, signature: string, repository: string, pubDate: string}} input
 */
export function buildManifest({ tag, notes, signature, repository, pubDate }) {
  const version = versionOfTag(tag);
  const entry = {
    signature: signature.trim(),
    url: assetUrl(repository, tag, installerName(version)),
  };
  return {
    version,
    notes,
    pub_date: pubDate,
    platforms: Object.fromEntries(
      WINDOWS_TARGETS.map((target) => [target, { ...entry }]),
    ),
  };
}

/**
 * Throws unless `manifest` is a well formed Windows feed for `tag`.
 * @param {any} manifest
 * @param {string} tag
 * @param {string} [repository] When given, the installer URL must be the one of that release.
 * @returns {void}
 */
export function assertWindowsManifest(manifest, tag, repository) {
  const version = versionOfTag(tag);
  if (!manifest || manifest.version !== version) {
    throw new Error(`manifest version ${manifest?.version} is not ${version}`);
  }
  const platforms = manifest.platforms ?? {};
  const foreign = Object.keys(platforms).filter(
    (key) => !key.startsWith("windows-"),
  );
  if (foreign.length > 0) {
    throw new Error(`non-Windows updater platforms: ${foreign.join(", ")}`);
  }
  for (const target of WINDOWS_TARGETS) {
    const entry = platforms[target];
    if (!entry?.url || !entry?.signature) {
      throw new Error(`missing updater platform ${target}`);
    }
    if (
      repository &&
      entry.url !== assetUrl(repository, tag, installerName(version))
    ) {
      throw new Error(`${target} does not point at the ${tag} installer`);
    }
  }
  if (
    platforms[WINDOWS_TARGETS[0]].url !== platforms[WINDOWS_TARGETS[1]].url ||
    platforms[WINDOWS_TARGETS[0]].signature !==
      platforms[WINDOWS_TARGETS[1]].signature
  ) {
    throw new Error("the Windows platform entries differ");
  }
}

/**
 * Checks the installer against the manifest's signature and the pubkey, and
 * that the signature was made for this file and this version.
 * @param {{installer: Buffer, fileName: string, manifest: any, tag: string, pubkey: string}} input
 * @returns {void}
 */
export function verifyInstaller({
  installer,
  fileName,
  manifest,
  tag,
  pubkey,
}) {
  const version = versionOfTag(tag);
  const signed = verifySignature({
    data: installer,
    signature: manifest.platforms[WINDOWS_TARGETS[0]].signature,
    pubkey,
  });
  if (signed.file !== fileName) {
    throw new Error(`signature is for ${signed.file}, not ${fileName}`);
  }
  if (signed.version !== version) {
    throw new Error(
      `signature is for version ${signed.version}, not ${version}`,
    );
  }
}

/** @param {string} configPath */
export function pubkeyOf(configPath) {
  const pubkey = JSON.parse(readFileSync(configPath, "utf8"))?.plugins?.updater
    ?.pubkey;
  if (typeof pubkey !== "string" || pubkey.length === 0) {
    throw new Error(`${configPath} has no updater pubkey`);
  }
  return pubkey;
}

/**
 * Checks what GitHub serves after a release: the Windows feed, its installer,
 * the checksum in the release notes, and that the macOS feed and the latest
 * release were not touched.
 * @param {{tag: string, repository: string, pubkey: string, fetchImpl?: typeof fetch, token?: string}} input
 * @returns {Promise<void>}
 */
export async function verifyPublic({
  tag,
  repository,
  pubkey,
  fetchImpl = fetch,
  token,
}) {
  const version = versionOfTag(tag);
  /** @param {string} url @param {Record<string, string>} [headers] */
  const get = async (url, headers = {}) => {
    const response = await fetchImpl(url, { headers, redirect: "follow" });
    if (!response.ok) throw new Error(`GET ${url} answered ${response.status}`);
    return response;
  };
  const api = {
    Accept: "application/vnd.github+json",
    "X-GitHub-Api-Version": "2022-11-28",
    ...(token ? { Authorization: `Bearer ${token}` } : {}),
  };

  const feedUrl = `https://github.com/${repository}/releases/download/${FEED_TAG}/latest.json`;
  const manifest = await (await get(feedUrl)).json();
  assertWindowsManifest(manifest, tag, repository);

  const installer = Buffer.from(
    await (await get(manifest.platforms[WINDOWS_TARGETS[0]].url)).arrayBuffer(),
  );
  verifyInstaller({
    installer,
    fileName: installerName(version),
    manifest,
    tag,
    pubkey,
  });

  const release = await (
    await get(
      `https://api.github.com/repos/${repository}/releases/tags/${tag}`,
      api,
    )
  ).json();
  if (!release.prerelease) throw new Error(`${tag} is not a pre-release`);
  const listed = CHECKSUM_LINE.exec(release.body ?? "")?.[1];
  const actual = createHash("sha256").update(installer).digest("hex");
  if (listed !== actual) {
    throw new Error("the SHA-256 in the release notes is not the installer's");
  }

  const macos = await (
    await get(
      `https://github.com/${repository}/releases/latest/download/latest.json`,
    )
  ).json();
  const foreign = Object.keys(macos.platforms ?? {}).filter(
    (key) => !key.startsWith("darwin-"),
  );
  if (foreign.length > 0) {
    throw new Error(`the macOS feed lists ${foreign.join(", ")}`);
  }
  if (typeof macos.version !== "string" || macos.version.includes("-")) {
    throw new Error(`the macOS feed announces ${macos.version}`);
  }
  const latest = await (
    await get(`https://api.github.com/repos/${repository}/releases/latest`, api)
  ).json();
  if (latest.prerelease || TAG_PATTERN.test(latest.tag_name ?? "")) {
    throw new Error(`the latest release is ${latest.tag_name}`);
  }
}

/** @param {string[]} argv */
function options(argv) {
  /** @type {Record<string, string>} */
  const values = {};
  for (let index = 0; index < argv.length; index += 2) {
    const key = argv[index];
    const value = argv[index + 1];
    if (!key?.startsWith("--") || value === undefined) {
      throw new Error(`bad argument ${key}`);
    }
    values[key.slice(2)] = value;
  }
  return values;
}

/** @param {Record<string, string>} values @param {string[]} names */
function required(values, names) {
  for (const name of names) {
    if (!values[name]) throw new Error(`--${name} is required`);
  }
}

/** @param {string[]} argv */
export async function main(argv) {
  const [command, ...rest] = argv;
  const values = options(rest);
  if (command === "notes") {
    required(values, ["version", "message", "installer", "out"]);
    writeFileSync(
      values.out,
      releaseNotes({
        version: values.version,
        message: readFileSync(values.message, "utf8"),
        checksum: createHash("sha256")
          .update(readFileSync(values.installer))
          .digest("hex"),
      }),
    );
    console.log(`wrote the notes for ${values.version}`);
  } else if (command === "build") {
    required(values, ["installer", "sig", "tag", "notes", "repository", "out"]);
    const manifest = buildManifest({
      tag: values.tag,
      notes: readFileSync(values.notes, "utf8"),
      signature: readFileSync(values.sig, "utf8"),
      repository: values.repository,
      pubDate: values["pub-date"] ?? new Date().toISOString(),
    });
    writeFileSync(values.out, `${JSON.stringify(manifest, null, 2)}\n`);
    console.log(`wrote the feed for ${manifest.version}`);
  } else if (command === "verify") {
    required(values, ["installer", "manifest", "tag", "config"]);
    const manifest = JSON.parse(readFileSync(values.manifest, "utf8"));
    assertWindowsManifest(manifest, values.tag);
    verifyInstaller({
      installer: readFileSync(values.installer),
      fileName: path.basename(values.installer),
      manifest,
      tag: values.tag,
      pubkey: pubkeyOf(values.config),
    });
    console.log(`the signature of ${versionOfTag(values.tag)} verifies`);
  } else if (command === "verify-public") {
    required(values, ["tag", "repository", "config"]);
    await verifyPublic({
      tag: values.tag,
      repository: values.repository,
      pubkey: pubkeyOf(values.config),
      token: process.env.GITHUB_TOKEN,
    });
    console.log(`the public feed for ${versionOfTag(values.tag)} is intact`);
  } else {
    throw new Error("usage: notes | build | verify | verify-public");
  }
}

if (
  process.argv[1] &&
  import.meta.url === pathToFileURL(process.argv[1]).href
) {
  main(process.argv.slice(2)).catch((error) => {
    console.error(error instanceof Error ? error.message : String(error));
    process.exit(1);
  });
}
