// Renders src-tauri/icons/dmg/background.svg, the installer window's
// background, to PNG at 1x and 2x and joins them into the TIFF that
// `bundle.macOS.dmg.background` ships, so Finder picks the sharp one on Retina.
//
//   bun scripts/render-dmg-background.ts
//
// macOS only (`tiffutil`). Needs Playwright's Chromium.
import { execFileSync } from "node:child_process";
import { readFileSync } from "node:fs";
import path from "node:path";
import { chromium } from "playwright";

const dir = path.join(import.meta.dir, "../src-tauri/icons/dmg");
const svg = readFileSync(path.join(dir, "background.svg"), "utf8");
const size = { width: 420, height: 560 };

const browser = await chromium.launch();
for (const [scale, name] of [
  [1, "background.png"],
  [2, "background@2x.png"],
] as const) {
  const page = await browser.newPage({
    viewport: size,
    deviceScaleFactor: scale,
  });
  await page.setContent(`<body style="margin:0">${svg}</body>`);
  await page.screenshot({
    path: path.join(dir, name),
    clip: { x: 0, y: 0, ...size },
  });
  await page.close();
}
await browser.close();

execFileSync(
  "tiffutil",
  [
    "-cathidpicheck",
    "background.png",
    "background@2x.png",
    "-out",
    "background.tiff",
  ],
  { cwd: dir, stdio: "inherit" },
);
