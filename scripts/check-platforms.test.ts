import { afterAll, describe, expect, test } from "bun:test";
import { spawnSync } from "node:child_process";
import {
  mkdirSync,
  mkdtempSync,
  readFileSync,
  rmSync,
  symlinkSync,
  unlinkSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import {
  PLATFORMS,
  bannedTextHits,
  checkMacos,
  checkWindows,
  isWindowsFile,
  runPlatforms,
  withoutWindowsBlocks,
} from "./check-platforms.ts";

const repoRoot = path.resolve(import.meta.dir, "..");
const scratch: string[] = [];

afterAll(() => {
  for (const directory of scratch) {
    rmSync(directory, { recursive: true, force: true });
  }
});

/** The repo as symlinks in a temp folder; a test replaces the files it changes. */
function cloneRepo(): string {
  const listed = spawnSync(
    "git",
    ["ls-files", "-z", "--cached", "--others", "--exclude-standard"],
    { cwd: repoRoot, encoding: "utf8" },
  );
  const root = mkdtempSync(path.join(tmpdir(), "check-platforms-"));
  scratch.push(root);
  for (const file of listed.stdout.split("\0").filter(Boolean)) {
    const target = path.join(root, file);
    mkdirSync(path.dirname(target), { recursive: true });
    symlinkSync(path.join(repoRoot, file), target);
  }
  return root;
}

function rewrite(
  root: string,
  file: string,
  change: (text: string) => string,
): void {
  const target = path.join(root, file);
  const text = readFileSync(target, "utf8");
  unlinkSync(target);
  writeFileSync(target, change(text));
}

describe("macOS section", () => {
  test("passes on the repository as it is", () => {
    expect(runPlatforms(repoRoot)).toEqual({
      failures: [],
      passed: [
        expect.stringMatching(/^macos \(/),
        expect.stringMatching(/^windows \(/),
      ],
    });
  });

  test("fails when a required macOS path is missing", () => {
    const root = cloneRepo();
    unlinkSync(path.join(root, "src-tauri/src/engine/local/fluid.rs"));
    expect(runPlatforms(root).failures).toContain(
      "[macos] required macOS asset is missing: src-tauri/src/engine/local/fluid.rs",
    );
  });

  test("fails when an obsolete path comes back", () => {
    const root = cloneRepo();
    writeFileSync(path.join(root, "src-tauri/src/portable.rs"), "");
    expect(runPlatforms(root).failures).toContain(
      "[macos] obsolete path remains: src-tauri/src/portable.rs",
    );
  });
});

describe("macOS contract", () => {
  const pinned: [string, string, (text: string) => string, string][] = [
    [
      "the bundle identifier",
      "src-tauri/tauri.conf.json",
      (text) => text.replace("com.dailyxplorer.murmur", "com.example.murmur"),
      "tauri.conf.json 'identifier'",
    ],
    [
      "the updater endpoint",
      "src-tauri/tauri.conf.json",
      (text) => text.replace("releases/latest/download", "releases/download"),
      "tauri.conf.json 'plugins.updater.endpoints'",
    ],
    [
      "the macOS bundle object",
      "src-tauri/tauri.conf.json",
      (text) =>
        text.replace('"hardenedRuntime": true', '"hardenedRuntime": false'),
      "tauri.conf.json 'bundle.macOS'",
    ],
    [
      "the entitlements",
      "src-tauri/Entitlements.plist",
      (text) => `${text}\n`,
      "src-tauri/Entitlements.plist changed",
    ],
    [
      "a signing secret of the release workflow",
      ".github/workflows/release.yml",
      (text) => text.replaceAll("MACOS_SIGNING_IDENTITY", "OTHER_IDENTITY"),
      "release.yml no longer holds 'secrets.MACOS_SIGNING_IDENTITY'",
    ],
    [
      "the environment of the release workflow",
      ".github/workflows/release.yml",
      (text) => text.replace('environment: "release-signing"', ""),
      "release.yml no longer holds 'environment: \"release-signing\"'",
    ],
    [
      "a signing step of the build workflow",
      ".github/workflows/build.yml",
      (text) =>
        text.replace(
          "Import stable signing certificate",
          "Import a certificate",
        ),
      "build.yml no longer holds '- name: Import stable signing certificate'",
    ],
    [
      "the Intel runner of the matrix",
      ".github/workflows/release.yml",
      (text) =>
        text.replace('platform: "macos-latest"', 'platform: "macos-15"'),
      'release.yml no longer holds \'platform: "macos-latest"',
    ],
  ];

  for (const [label, file, change, expected] of pinned) {
    test(`fails when ${label} changes`, () => {
      const root = cloneRepo();
      rewrite(root, file, change);
      const failures = checkMacos(root).failures.join("\n");
      expect(failures).toContain("macOS contract");
      expect(failures).toContain(expected);
    });
  }
});

describe("platform guard of lib.rs", () => {
  const librs = "src-tauri/src/lib.rs";
  const macosOnly = [
    '#[cfg(not(target_os = "macos"))]',
    'compile_error!("Murmur supports macOS only.");',
  ].join("\n");
  const both = [
    '#[cfg(not(any(target_os = "macos", target_os = "windows")))]',
    'compile_error!("Murmur supports macOS and Windows only.");',
  ].join("\n");

  test("accepts the macOS-only guard", () => {
    const root = cloneRepo();
    rewrite(root, librs, (text) => text.replace(both, macosOnly));
    expect(checkMacos(root).failures).toEqual([]);
  });

  test("accepts the macOS and Windows guard", () => {
    expect(checkMacos(cloneRepo()).failures).toEqual([]);
  });

  test("rejects a missing or altered guard", () => {
    for (const replacement of [
      "",
      both.replace("macOS and Windows only", "everything"),
    ]) {
      const root = cloneRepo();
      rewrite(root, librs, (text) => text.replace(both, replacement));
      expect(checkMacos(root).failures).toContain(
        "the Rust crate does not reject non-macOS targets",
      );
    }
  });
});

describe("Windows text in Rust", () => {
  const banned = [
    "PasteMethod::ShiftInsert",
    "WindowsMicrophonePermissionStatus",
  ];
  const shared = "src-tauri/src/clipboard.rs";

  test("passes in files named for Windows", () => {
    for (const file of [
      "src-tauri/src/meetings/power_windows.rs",
      "src-tauri/src/durable/windows.rs",
      "src-tauri/src/input/win32.rs",
      "src-tauri/src/uia.rs",
      "src-tauri/src/uia/tree.rs",
      "src-tauri/src/uia_probe.rs",
    ]) {
      expect(isWindowsFile(file)).toBe(true);
      expect(bannedTextHits(file, "PasteMethod::ShiftInsert", banned)).toEqual(
        [],
      );
    }
  });

  test("fails in a shared file with no cfg", () => {
    expect(isWindowsFile(shared)).toBe(false);
    expect(isWindowsFile("src-tauri/src/windowing.rs")).toBe(false);
    expect(
      bannedTextHits(shared, "let m = PasteMethod::ShiftInsert;", banned),
    ).toEqual(["PasteMethod::ShiftInsert"]);
  });

  test("passes inside a Windows-only item and fails right after it", () => {
    const source = [
      "fn shared() {}",
      '#[cfg(target_os = "windows")]',
      "fn paste() {",
      "    let method = PasteMethod::ShiftInsert;",
      '    let brace = "}";',
      "}",
      '#[cfg(target_os = "windows")]',
      "use crate::WindowsMicrophonePermissionStatus;",
      "fn after() { let _ = PasteMethod::ShiftInsert; }",
    ].join("\n");
    expect(bannedTextHits(shared, source, banned)).toEqual([
      "PasteMethod::ShiftInsert",
    ]);
    expect(withoutWindowsBlocks(source).split("\n")).toEqual([
      "fn shared() {}",
      "",
      "",
      "",
      "",
      "",
      "",
      "",
      "fn after() { let _ = PasteMethod::ShiftInsert; }",
    ]);
  });

  test("passes in a file that is Windows-only by an inner attribute", () => {
    expect(
      bannedTextHits(
        shared,
        '#![cfg(target_os = "windows")]\nPasteMethod::ShiftInsert',
        banned,
      ),
    ).toEqual([]);
  });

  test("a cfg(not(windows)) block is compiled for macOS", () => {
    const source =
      '#[cfg(not(target_os = "windows"))]\nfn paste() { PasteMethod::ShiftInsert }';
    expect(bannedTextHits(shared, source, banned)).toEqual([
      "PasteMethod::ShiftInsert",
    ]);
  });

  test("the check reports it from a real shared file", () => {
    const root = cloneRepo();
    rewrite(
      root,
      shared,
      (text) => `${text}\nconst _: &str = "PasteMethod::ShiftInsert";\n`,
    );
    expect(runPlatforms(root).failures).toContain(
      `[macos] legacy Rust symbol 'PasteMethod::ShiftInsert' remains: ${shared}`,
    );
  });
});

describe("Windows section", () => {
  const windows = PLATFORMS.find((platform) => platform.name === "windows")!;

  function fixture(overlay: boolean): string {
    const root = mkdtempSync(path.join(tmpdir(), "check-platforms-win-"));
    scratch.push(root);
    if (overlay) {
      mkdirSync(path.join(root, "src-tauri"));
      writeFileSync(path.join(root, "src-tauri/tauri.windows.conf.json"), "{}");
    }
    return root;
  }

  test("is a no-op without the overlay and active with it", () => {
    expect(windows.enabled(fixture(false))).toBe(false);
    expect(windows.enabled(fixture(true))).toBe(true);
    expect(windows.enabled(repoRoot)).toBe(true);
  });

  test("names the platform in its failures", () => {
    const root = fixture(true);
    expect(checkWindows(root, ["src-tauri/src/uia.rs"]).failures).toEqual([
      "required Windows asset is missing: src-tauri/src/uia.rs",
    ]);
    mkdirSync(path.join(root, "src-tauri/src"));
    writeFileSync(path.join(root, "src-tauri/src/uia.rs"), "");
    expect(checkWindows(root, ["src-tauri/src/uia.rs"]).failures).toEqual([]);
  });

  test("the repository holds every required Windows path", () => {
    expect(checkWindows(repoRoot).failures).toEqual([]);
  });

  test("the Windows updater reads its own feed in passive mode", () => {
    const root = cloneRepo();
    rewrite(root, "src-tauri/tauri.windows.conf.json", (text) =>
      text
        .replace("windows-beta/latest.json", "latest/download/latest.json")
        .replace('"passive"', '"basicUi"')
        .replace(
          '"createUpdaterArtifacts": false',
          '"createUpdaterArtifacts": true',
        ),
    );
    expect(checkWindows(root).failures).toEqual([
      "the Windows config builds updater artifacts: a release build turns them on",
      "the Windows updater does not read exactly the windows-beta feed",
      "the Windows updater does not install in passive mode",
    ]);
  });

  test("a tree without the Windows workflow fails", () => {
    expect(checkWindows(fixture(true)).failures).toContain(
      "required Windows asset is missing: .github/workflows/windows.yml",
    );
  });
});

describe("platform branches", () => {
  test("rejects a branch for another platform", () => {
    const root = cloneRepo();
    rewrite(
      root,
      "src-tauri/src/clipboard.rs",
      (text) => `#[cfg(target_os = "linux")]\nfn other() {}\n${text}`,
    );
    expect(checkMacos(root).failures).toContain(
      "branch for unsupported platform 'linux': src-tauri/src/clipboard.rs",
    );
  });

  test("rejects Cargo dependencies for a target that is neither platform", () => {
    const root = cloneRepo();
    rewrite(
      root,
      "src-tauri/Cargo.toml",
      (text) =>
        `${text}\n[target.'cfg(not(target_os = "macos"))'.dependencies]\nlibc = "0.2"\n`,
    );
    expect(checkMacos(root).failures).toContain(
      `Cargo dependencies for an unsupported target: not(target_os = "macos")`,
    );
  });
});
