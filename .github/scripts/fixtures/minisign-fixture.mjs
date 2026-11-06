/**
 * @fileoverview Throw-away signing key for tests: writes the same minisign
 * text the Tauri CLI writes (Ed25519 over the BLAKE2b-512 prehash, a global
 * signature over the trusted comment), with Node's `crypto` only.
 */
import {
  createHash,
  generateKeyPairSync,
  randomBytes,
  sign as edSign,
} from "node:crypto";

/**
 * @param {Buffer | string} value
 * @returns {string}
 */
function base64(value) {
  return Buffer.from(value).toString("base64");
}

/**
 * @returns {{
 *   pubkey: string,
 *   keyIdHex: string,
 *   sign: (data: Buffer, fields: {file: string, version?: string, timestamp?: string}) => string,
 * }}
 *   `pubkey` goes where `plugins.updater.pubkey` goes; `sign` returns what a
 *   `.sig` file and a manifest `signature` hold.
 */
export function createTestKey() {
  const { publicKey, privateKey } = generateKeyPairSync("ed25519");
  const raw = Buffer.from(publicKey.export({ format: "jwk" }).x, "base64url");
  const keyId = randomBytes(8);
  const keyIdHex = Buffer.from(keyId).reverse().toString("hex").toUpperCase();

  const pubkey = base64(
    `untrusted comment: minisign public key: ${keyIdHex}\n${base64(Buffer.concat([Buffer.from("Ed"), keyId, raw]))}\n`,
  );

  return {
    pubkey,
    keyIdHex,
    sign(data, { file, version, timestamp = "1793951366" }) {
      const prehash = createHash("blake2b512").update(data).digest();
      const signature = edSign(null, prehash, privateKey);
      const trusted = [`timestamp:${timestamp}`, `file:${file}`]
        .concat(version === undefined ? [] : [`version:${version}`])
        .join("\t");
      const global = edSign(
        null,
        Buffer.concat([signature, Buffer.from(trusted)]),
        privateKey,
      );
      return base64(
        [
          "untrusted comment: signature from tauri secret key",
          base64(Buffer.concat([Buffer.from("ED"), keyId, signature])),
          `trusted comment: ${trusted}`,
          base64(global),
          "",
        ].join("\n"),
      );
    },
  };
}
