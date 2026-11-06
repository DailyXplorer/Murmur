/**
 * @fileoverview Preload (`node --import`) that replaces `fetch` and `Date` so
 * publish-updater-manifest.mjs runs offline against a described release.
 *
 * FAKE_GITHUB_RELEASE: path to JSON `{release: {tag_name, body}, assets: [{id, name}], signatures: {<id>: text}}`.
 * FAKE_GITHUB_UPLOAD: file that receives the body of the `latest.json` upload.
 * FAKE_GITHUB_NOW: the instant `new Date()` returns.
 */
import { readFileSync, writeFileSync } from "node:fs";

const fixture = JSON.parse(
  readFileSync(process.env.FAKE_GITHUB_RELEASE, "utf8"),
);
const now = process.env.FAKE_GITHUB_NOW;

if (now) {
  const RealDate = Date;
  globalThis.Date = class FrozenDate extends RealDate {
    constructor(...args) {
      super(...(args.length === 0 ? [now] : args));
    }
    static now() {
      return new RealDate(now).getTime();
    }
  };
}

/**
 * @param {unknown} body
 * @param {number} [status]
 * @returns {Response}
 */
function json(body, status = 200) {
  return new Response(JSON.stringify(body), { status });
}

globalThis.fetch = async (input, init = {}) => {
  const url = new URL(String(input));
  const method = init.method ?? "GET";
  const path = url.pathname;

  if (url.host === "uploads.github.com" && method === "POST") {
    writeFileSync(process.env.FAKE_GITHUB_UPLOAD, String(init.body));
    return json({});
  }
  if (method === "DELETE") return new Response(null, { status: 204 });
  if (/\/releases\/\d+$/.test(path)) return json(fixture.release);
  if (/\/releases\/\d+\/assets$/.test(path)) {
    return json(url.searchParams.get("page") === "1" ? fixture.assets : []);
  }
  const asset = /\/releases\/assets\/(\d+)$/.exec(path);
  if (asset && fixture.signatures[asset[1]] !== undefined) {
    return new Response(fixture.signatures[asset[1]]);
  }
  return json({ message: `unexpected ${method} ${path}` }, 404);
};
