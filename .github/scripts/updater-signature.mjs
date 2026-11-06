/**
 * @fileoverview Offline check of the signatures in `latest.json`, with Node's
 * `crypto` only. The Tauri CLI signs each updater artifact in the minisign
 * "prehashed" form: Ed25519 over the BLAKE2b-512 hash of the file, plus a
 * global signature that covers the trusted comment
 * (`timestamp:<n>\tfile:<name>\tversion:<v>`).
 *
 * Text formats, all base64 encoded once more in the Tauri config and in the
 * `signature` field of the manifest:
 * - public key: "untrusted comment: ...", then base64("Ed" + key id 8 + key 32);
 * - signature: "untrusted comment: ...", base64("ED" + key id 8 + signature 64),
 *   "trusted comment: ...", base64(global signature 64).
 */
import { createHash, createPublicKey, verify } from "node:crypto";
import { readFile } from "node:fs/promises";
import path from "node:path";

const SPKI_PREFIX = Buffer.from("302a300506032b6570032100", "hex");
const KEY_ALGORITHM = "Ed";
const PREHASHED_ALGORITHM = "ED";
const TRUSTED_PREFIX = "trusted comment: ";

/**
 * @param {string} value
 * @param {string} what
 * @returns {Buffer}
 */
function decodeBase64(value, what) {
  const text = value.trim();
  if (!/^[A-Za-z0-9+/]+={0,2}$/.test(text)) {
    throw new Error(`${what} is not base64`);
  }
  return Buffer.from(text, "base64");
}

/**
 * @param {string} base64Text
 * @param {string} what
 * @returns {string[]}
 */
function textLines(base64Text, what) {
  return decodeBase64(base64Text, what)
    .toString("utf8")
    .replaceAll("\r\n", "\n")
    .split("\n");
}

/**
 * @param {string} pubkey The `pubkey` of `tauri.conf.json`.
 * @returns {{keyId: Buffer, key: Buffer}}
 */
export function decodePublicKey(pubkey) {
  const bytes = decodeBase64(
    textLines(pubkey, "public key")[1] ?? "",
    "public key line",
  );
  if (bytes.length !== 42 || bytes.toString("latin1", 0, 2) !== KEY_ALGORITHM) {
    throw new Error("public key is not a minisign Ed25519 key");
  }
  return { keyId: bytes.subarray(2, 10), key: bytes.subarray(10) };
}

/**
 * @param {string} encoded A `.sig` file, or a manifest `signature`.
 * @returns {{algorithm: string, keyId: Buffer, signature: Buffer, trustedComment: string, globalSignature: Buffer}}
 */
export function decodeSignature(encoded) {
  const [, signatureLine, commentLine, globalLine] = textLines(
    encoded,
    "signature",
  );
  if (!commentLine?.startsWith(TRUSTED_PREFIX)) {
    throw new Error("signature has no trusted comment");
  }
  const bytes = decodeBase64(signatureLine ?? "", "signature line");
  const globalSignature = decodeBase64(
    globalLine ?? "",
    "global signature line",
  );
  if (bytes.length !== 74 || globalSignature.length !== 64) {
    throw new Error("signature has the wrong size");
  }
  return {
    algorithm: bytes.toString("latin1", 0, 2),
    keyId: bytes.subarray(2, 10),
    signature: bytes.subarray(10),
    trustedComment: commentLine.slice(TRUSTED_PREFIX.length),
    globalSignature,
  };
}

/**
 * @param {Buffer} key
 * @param {Buffer} message
 * @param {Buffer} signature
 * @returns {boolean}
 */
function ed25519Verify(key, message, signature) {
  const publicKey = createPublicKey({
    key: Buffer.concat([SPKI_PREFIX, key]),
    format: "der",
    type: "spki",
  });
  return verify(null, message, publicKey, signature);
}

/**
 * Checks `data` against `signature` made by the key `pubkey`, then returns the
 * trusted comment fields the global signature covers.
 * @param {{data: Buffer, signature: string, pubkey: string}} input
 * @returns {Record<string, string>} `timestamp`, `file`, `version` when present.
 */
export function verifySignature({ data, signature, pubkey }) {
  const publicKey = decodePublicKey(pubkey);
  const decoded = decodeSignature(signature);

  if (!decoded.keyId.equals(publicKey.keyId)) {
    throw new Error(
      `signed by key ${decoded.keyId.toString("hex")}, not ${publicKey.keyId.toString("hex")}`,
    );
  }
  if (decoded.algorithm !== PREHASHED_ALGORITHM) {
    throw new Error("signature is not the prehashed minisign kind");
  }
  const prehash = createHash("blake2b512").update(data).digest();
  if (!ed25519Verify(publicKey.key, prehash, decoded.signature)) {
    throw new Error("signature does not match the file");
  }
  const covered = Buffer.concat([
    decoded.signature,
    Buffer.from(decoded.trustedComment, "utf8"),
  ]);
  if (!ed25519Verify(publicKey.key, covered, decoded.globalSignature)) {
    throw new Error("trusted comment does not match its signature");
  }

  /** @type {Record<string, string>} */
  const fields = {};
  for (const field of decoded.trustedComment.split("\t")) {
    const colon = field.indexOf(":");
    if (colon > 0) fields[field.slice(0, colon)] ??= field.slice(colon + 1);
  }
  return fields;
}

/**
 * Same rule as the updater plugin: `v1.2.3` and `1.2.3` are one version.
 * @param {string} version
 * @returns {string}
 */
function bareVersion(version) {
  return version.replace(/^v/, "");
}

/**
 * The names the signed `file:` field may carry for an asset. The Tauri bundler
 * signs the Mac archive as `<product>.app.tar.gz`; the release upload then adds
 * the architecture to the asset name (`<product>_aarch64.app.tar.gz`). The
 * Windows installer keeps its name.
 * @param {string} key
 * @param {string} assetName
 * @returns {string[]}
 */
export function signedFileNames(key, assetName) {
  if (!key.startsWith("darwin-")) return [assetName];
  const bundled = assetName.replace(
    /_(?:aarch64|arm64|x86_64|amd64|x64)(?=\.app\.tar\.gz$)/i,
    "",
  );
  return [assetName, bundled];
}

/**
 * Verifies every entry of a manifest against the file it points at, found by
 * name in `directory`. The signed `version` and `file` fields must match the
 * release version and the asset name (see `signedFileNames`).
 * @param {{platforms: Record<string, {url: string, signature: string}>}} manifest
 * @param {{directory: string, pubkey: string, version: string}} input
 * @returns {Promise<string[]>} The keys that were verified.
 */
export async function verifyManifestSignatures(manifest, input) {
  const { directory, pubkey, version } = input;
  const keys = Object.keys(manifest.platforms).sort();
  for (const key of keys) {
    const { url, signature } = manifest.platforms[key];
    const name = decodeURIComponent(url.slice(url.lastIndexOf("/") + 1));
    if (name !== path.basename(name)) {
      throw new Error(`${key}: ${name} is not a plain file name`);
    }
    let data;
    try {
      data = await readFile(path.join(directory, name));
    } catch {
      throw new Error(`${key}: ${name} is not in ${directory}`);
    }
    let fields;
    try {
      fields = verifySignature({ data, signature, pubkey });
    } catch (error) {
      throw new Error(`${key}: ${name}: ${error.message}`);
    }
    if (!signedFileNames(key, name).includes(fields.file)) {
      throw new Error(
        `${key}: signed file name ${fields.file ?? "(none)"} does not match ${name}`,
      );
    }
    if (
      fields.version === undefined ||
      bareVersion(fields.version) !== bareVersion(version)
    ) {
      throw new Error(
        `${key}: signed version ${fields.version ?? "(none)"} is not ${version}`,
      );
    }
  }
  return keys;
}
