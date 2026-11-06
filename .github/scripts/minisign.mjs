/**
 * @fileoverview Verify-only minisign, enough to check what the Tauri CLI signs
 * for the updater. No dependency: Node's `crypto` has Ed25519 and BLAKE2b-512.
 *
 * Formats (https://jedisct1.github.io/minisign/):
 * - public key: text of two lines, the second the base64 of
 *   "Ed" + key id (8 bytes) + Ed25519 public key (32 bytes);
 * - signature: text of four lines: an untrusted comment, the base64 of
 *   algorithm (2 bytes) + key id (8 bytes) + signature (64 bytes), a trusted
 *   comment, and the base64 of the global signature (64 bytes) over the
 *   signature followed by the trusted comment.
 * The Tauri config and the `signature` field of `latest.json` hold those texts
 * base64 encoded once more.
 */
import { createHash, createPublicKey, verify } from "node:crypto";

const ED25519_SPKI_PREFIX = Buffer.from("302a300506032b6570032100", "hex");
const PUBLIC_KEY_ALGORITHM = "Ed";
/** Ed25519 over the BLAKE2b-512 hash of the file: what the Tauri CLI writes. */
const PREHASHED_ALGORITHM = "ED";
const TRUSTED_COMMENT_PREFIX = "trusted comment: ";

/** @param {string} text */
function lines(text) {
  return text.replaceAll("\r\n", "\n").split("\n");
}

/** @param {string} value @param {string} what */
function fromBase64(value, what) {
  const trimmed = value.trim();
  if (!/^[A-Za-z0-9+/]+={0,2}$/.test(trimmed)) {
    throw new Error(`${what} is not base64`);
  }
  return Buffer.from(trimmed, "base64");
}

/**
 * @param {string} encoded The `pubkey` of the Tauri config.
 * @returns {{keyId: Buffer, key: Buffer}}
 */
export function decodePublicKey(encoded) {
  const [, keyLine] = lines(fromBase64(encoded, "public key").toString("utf8"));
  const bytes = fromBase64(keyLine ?? "", "public key line");
  if (
    bytes.length !== 42 ||
    bytes.toString("latin1", 0, 2) !== PUBLIC_KEY_ALGORITHM
  ) {
    throw new Error("public key is not a minisign Ed25519 key");
  }
  return { keyId: bytes.subarray(2, 10), key: bytes.subarray(10) };
}

/**
 * @param {string} encoded The content of a `.sig` file, or a manifest `signature`.
 * @returns {{algorithm: string, keyId: Buffer, signature: Buffer, trustedComment: string, globalSignature: Buffer}}
 */
export function decodeSignature(encoded) {
  const text = fromBase64(encoded, "signature").toString("utf8");
  const [, signatureLine, commentLine, globalLine] = lines(text);
  if (!commentLine?.startsWith(TRUSTED_COMMENT_PREFIX)) {
    throw new Error("signature has no trusted comment");
  }
  const bytes = fromBase64(signatureLine ?? "", "signature line");
  const globalSignature = fromBase64(globalLine ?? "", "global signature line");
  if (bytes.length !== 74 || globalSignature.length !== 64) {
    throw new Error("signature has the wrong size");
  }
  return {
    algorithm: bytes.toString("latin1", 0, 2),
    keyId: bytes.subarray(2, 10),
    signature: bytes.subarray(10),
    trustedComment: commentLine.slice(TRUSTED_COMMENT_PREFIX.length),
    globalSignature,
  };
}

/** @param {Buffer} key @param {Buffer} message @param {Buffer} signature */
function ed25519Verify(key, message, signature) {
  const publicKey = createPublicKey({
    key: Buffer.concat([ED25519_SPKI_PREFIX, key]),
    format: "der",
    type: "spki",
  });
  return verify(null, message, publicKey, signature);
}

/**
 * Checks `data` against a signature made by the key `pubkey` and returns the
 * fields of the trusted comment, which the global signature covers.
 * @param {{data: Buffer, signature: string, pubkey: string}} input
 * @returns {{timestamp?: string, file?: string, version?: string}}
 */
export function verifySignature({ data, signature, pubkey }) {
  const publicKey = decodePublicKey(pubkey);
  const decoded = decodeSignature(signature);

  if (!decoded.keyId.equals(publicKey.keyId)) {
    throw new Error(
      `signature was made by key ${decoded.keyId.toString("hex")}, not ${publicKey.keyId.toString("hex")}`,
    );
  }
  if (decoded.algorithm !== PREHASHED_ALGORITHM) {
    throw new Error("signature is not the prehashed minisign kind");
  }
  const prehash = createHash("blake2b512").update(data).digest();
  if (!ed25519Verify(publicKey.key, prehash, decoded.signature)) {
    throw new Error("signature does not match the file");
  }
  const signedComment = Buffer.concat([
    decoded.signature,
    Buffer.from(decoded.trustedComment, "utf8"),
  ]);
  if (!ed25519Verify(publicKey.key, signedComment, decoded.globalSignature)) {
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
