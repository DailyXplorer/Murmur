import { createHash } from "node:crypto";
import { existsSync, readFileSync, readdirSync } from "node:fs";
import path from "node:path";
import { isDeepStrictEqual } from "node:util";

const defaultRoot = path.resolve(import.meta.dir, "..");

export interface PlatformResult {
  failures: string[];
  summary: string;
}

export interface Platform {
  name: string;
  enabled(root: string): boolean;
  check(root: string): PlatformResult;
}

function collectFiles(
  root: string,
  directory: string,
  extension: string,
): string[] {
  const absoluteDirectory = path.join(root, directory);
  return readdirSync(absoluteDirectory, { withFileTypes: true }).flatMap(
    (entry) => {
      const relative = path.join(directory, entry.name);
      if (entry.isDirectory()) return collectFiles(root, relative, extension);
      return entry.name.endsWith(extension) ? [relative] : [];
    },
  );
}

function read(root: string, relativePath: string): string {
  return readFileSync(path.join(root, relativePath), "utf8");
}

// ---------------------------------------------------------------------------
// Rust platform guard and the Windows carve-out
// ---------------------------------------------------------------------------

const macosOnlyGuard = [
  '#[cfg(not(target_os = "macos"))]',
  'compile_error!("Murmur supports macOS only.");',
].join("\n");
// The guard the merged macOS and Windows tree carries. Either one is accepted
// until the merge lanes switch lib.rs to the second.
const supportedTargets = new Set(["macos", "windows"]);
const macosAndWindowsGuard = [
  '#[cfg(not(any(target_os = "macos", target_os = "windows")))]',
  'compile_error!("Murmur supports macOS and Windows only.");',
].join("\n");
const rustGuards = [macosOnlyGuard, macosAndWindowsGuard];

export function hasRustGuard(librs: string): boolean {
  return rustGuards.some((guard) => librs.includes(guard));
}

function withoutRustGuards(source: string): string {
  return rustGuards.reduce((text, guard) => text.replace(guard, ""), source);
}

/**
 * A file compiled for Windows only, by name: `*_windows.rs`, `windows.rs`,
 * `win32.rs`, `uia*` (a file or a directory), at any depth of the path.
 */
export function isWindowsFile(relativePath: string): boolean {
  return relativePath
    .split(/[\\/]/)
    .map((segment) => segment.replace(/\.rs$/, ""))
    .some((segment) => /^(windows|win32|uia.*|.*_windows)$/.test(segment));
}

const windowsCfgAttribute =
  /^\s*#(!?)\[cfg\((?!.*not\()[^\n]*target_os = "windows"/;

function codeOnly(line: string): string {
  return line.replace(/"(?:[^"\\]|\\.)*"/g, '""').replace(/\/\/.*$/, "");
}

/**
 * The source with every item under `#[cfg(target_os = "windows")]` (and the
 * whole file under `#![cfg(target_os = "windows")]`) blanked out, line by
 * line so the line numbers stay. Textual: an item ends at its closing brace or
 * at the first `;` outside braces. A `cfg(not(...))` is never Windows-only.
 */
export function withoutWindowsBlocks(source: string): string {
  const lines = source.split("\n");
  const kept = [...lines];
  for (let start = 0; start < lines.length; start++) {
    const attribute = windowsCfgAttribute.exec(lines[start]);
    if (!attribute) continue;
    if (attribute[1] === "!") return "";
    let depth = 0;
    let end = start;
    let opened = false;
    for (let index = start + 1; index < lines.length; index++) {
      end = index;
      const code = codeOnly(lines[index]);
      if (/^\s*(#\[|\/\/)/.test(lines[index]) && !opened) continue;
      for (const character of code) {
        if (character === "{") {
          depth++;
          opened = true;
        } else if (character === "}") depth--;
      }
      if (opened ? depth <= 0 : code.includes(";")) break;
    }
    for (let index = start; index <= end; index++) kept[index] = "";
    start = end;
  }
  return kept.join("\n");
}

/**
 * Banned text found in code compiled for macOS: nothing from a Windows file,
 * nothing inside a Windows-only item.
 */
export function bannedTextHits(
  relativePath: string,
  source: string,
  banned: string[],
): string[] {
  if (isWindowsFile(relativePath)) return [];
  const macosSource = withoutWindowsBlocks(source);
  return banned.filter((text) => macosSource.includes(text));
}

// ---------------------------------------------------------------------------
// macOS
// ---------------------------------------------------------------------------

const obsoletePaths = [
  ".cargo/config.toml",
  "src-tauri/capabilities/desktop.json",
  "src-tauri/icons/android",
  "src-tauri/icons/ios",
  "src-tauri/icons/64x64.png",
  "src-tauri/icons/logo.png",
  "src-tauri/icons/Square107x107Logo.png",
  "src-tauri/icons/Square142x142Logo.png",
  "src-tauri/icons/Square150x150Logo.png",
  "src-tauri/icons/Square284x284Logo.png",
  "src-tauri/icons/Square30x30Logo.png",
  "src-tauri/icons/Square310x310Logo.png",
  "src-tauri/icons/Square44x44Logo.png",
  "src-tauri/icons/Square71x71Logo.png",
  "src-tauri/icons/Square89x89Logo.png",
  "src-tauri/icons/StoreLogo.png",
  "src-tauri/icons/tray/idle.svg",
  "src-tauri/icons/tray/recording.svg",
  "src-tauri/icons/tray/transcribing.svg",
  "src-tauri/resources/default_settings.json",
  "src-tauri/resources/murmur.png",
  "src-tauri/resources/recording.png",
  "src-tauri/resources/transcribing.png",
  "src-tauri/resources/tray_idle.png",
  "src-tauri/resources/tray_idle_dark.png",
  "src-tauri/resources/tray_recording.png",
  "src-tauri/resources/tray_recording_dark.png",
  "src-tauri/resources/tray_transcribing.png",
  "src-tauri/resources/tray_transcribing_dark.png",
  "src-tauri/src/audio_toolkit/utils.rs",
  "src-tauri/src/memory.rs",
  "src-tauri/src/portable.rs",
  "src/components/ui/Alert.tsx",
  "src/components/ui/Badge.tsx",
  "src/components/ui/Select.tsx",
  "src/hooks/useOsType.ts",
  "src/hooks/useMeetingModels.ts",
  "src-tauri/capabilities/default.json",
];

const requiredPaths = [
  "src-tauri/icons/icon.png",
  "src-tauri/icons/tray/tray.svg",
  "src-tauri/resources/tray.png",
  "src-tauri/capabilities/main.json",
  "src-tauri/capabilities/recording-overlay.json",
  "src-tauri/capabilities/call-prompt.json",
  "src-tauri/permissions/default.toml",
  "src-tauri/src/engine/mod.rs",
  "src-tauri/src/engine/parakeet_languages.rs",
  "src-tauri/src/engine/local/mod.rs",
  "src-tauri/src/engine/local/session.rs",
  "src-tauri/src/engine/local/fluid.rs",
  "src-tauri/src/engine/local/cpu.rs",
  "src-tauri/src/audio_toolkit/pauses.rs",
  "src-tauri/src/local_models/mod.rs",
  "src-tauri/src/local_models/manifest.rs",
  "src-tauri/src/local_models/pinned.rs",
  "src-tauri/src/local_models/download.rs",
  "src-tauri/src/local_models/verify.rs",
  "src-tauri/src/commands/local_models.rs",
  "src/hooks/useLocalModel.ts",
];

// Windows text that must not be compiled for macOS. It is allowed in a
// Windows file and inside a `cfg(target_os = "windows")` item (`bannedTextHits`).
const forbiddenRustText = [
  "TypingTool",
  "portable::",
  "mod portable",
  "PasteMethod::ShiftInsert",
  "PasteMethod::CtrlShiftV",
  "PasteMethod::ExternalScript",
  "change_typing_tool_setting",
  "change_external_script_path_setting",
  "get_available_typing_tools",
  "WindowsMicrophonePermissionStatus",
  "get_windows_microphone_permission_status",
  "open_microphone_privacy_settings",
  "get_microphone_mode",
  "get_selected_microphone",
  "get_selected_output_device",
  "get_clamshell_microphone",
  "trigger_update_check",
];

type Capability = {
  identifier: string;
  windows: string[];
  permissions: unknown[];
};

function capabilityProblems(
  root: string,
  relativePath: string,
  identifier: string,
  windows: string[],
  expectedPermissions: string[],
): string[] {
  const capability = JSON.parse(read(root, relativePath)) as Capability;
  const actualPermissions = capability.permissions
    .filter(
      (permission): permission is string => typeof permission === "string",
    )
    .sort();
  if (
    capability.identifier !== identifier ||
    capability.windows.join("\n") !== windows.join("\n") ||
    capability.permissions.length !== expectedPermissions.length ||
    actualPermissions.join("\n") !== expectedPermissions.sort().join("\n")
  ) {
    return [`unexpected Tauri capability scope: ${relativePath}`];
  }
  return [];
}

// ---------------------------------------------------------------------------
// The macOS contract: scripts/fixtures/macos-contract.json
// ---------------------------------------------------------------------------

export const contractPath = "scripts/fixtures/macos-contract.json";

interface Pin<T> {
  value: T;
  why: string;
}

interface WorkflowFact {
  text?: string;
  regex?: string;
  why: string;
}

interface MacosContract {
  tauriConf: Record<string, Pin<unknown>>;
  fileHashes: Record<string, { sha256: string; why: string }>;
  workflows: Record<string, WorkflowFact[]>;
}

function jsonPath(value: unknown, dotted: string): unknown {
  return dotted
    .split(".")
    .reduce<unknown>(
      (node, key) =>
        node !== null && typeof node === "object"
          ? (node as Record<string, unknown>)[key]
          : undefined,
      value,
    );
}

/**
 * Compare the macOS-relevant fields of the build and release files with the
 * golden file. A value that differs is a deliberate edit of the golden file
 * with a reason, or a regression: it is never silent.
 */
export function contractProblems(root: string): string[] {
  const contract = JSON.parse(read(root, contractPath)) as MacosContract;
  const problems: string[] = [];

  const tauriConf = JSON.parse(read(root, "src-tauri/tauri.conf.json"));
  for (const [dotted, pin] of Object.entries(contract.tauriConf)) {
    if (!isDeepStrictEqual(jsonPath(tauriConf, dotted), pin.value)) {
      problems.push(
        `macOS contract: tauri.conf.json '${dotted}' differs from ${contractPath} (${pin.why})`,
      );
    }
  }

  for (const [file, pin] of Object.entries(contract.fileHashes)) {
    const absolute = path.join(root, file);
    const actual = existsSync(absolute)
      ? createHash("sha256").update(readFileSync(absolute)).digest("hex")
      : "missing";
    if (actual !== pin.sha256) {
      problems.push(
        `macOS contract: ${file} changed, sha256 ${actual} (${pin.why})`,
      );
    }
  }

  for (const [file, facts] of Object.entries(contract.workflows)) {
    const relativePath = path.join(".github/workflows", file);
    if (!existsSync(path.join(root, relativePath))) {
      problems.push(`macOS contract: ${relativePath} is missing`);
      continue;
    }
    const source = read(root, relativePath);
    for (const fact of facts) {
      const present = fact.regex
        ? new RegExp(fact.regex).test(source)
        : source.includes(fact.text ?? "");
      if (!present) {
        problems.push(
          `macOS contract: ${relativePath} no longer holds '${fact.text ?? fact.regex}' (${fact.why})`,
        );
      }
    }
  }

  return problems;
}

// Target tables of Cargo.toml: by OS for the two platforms, by architecture
// for the CPU engine of Intel Macs and Windows (never by "not macOS").
const allowedCargoTargets = new Set([
  "unix",
  'target_os = "macos"',
  'target_os = "windows"',
  'target_arch = "x86_64"',
  'all(target_os = "windows", target_arch = "x86_64")',
]);

export function checkMacos(root: string = defaultRoot): PlatformResult {
  const failures: string[] = [];

  for (const relativePath of obsoletePaths) {
    if (existsSync(path.join(root, relativePath))) {
      failures.push(`obsolete path remains: ${relativePath}`);
    }
  }

  for (const relativePath of requiredPaths) {
    if (!existsSync(path.join(root, relativePath))) {
      failures.push(`required macOS asset is missing: ${relativePath}`);
    }
  }

  const rustFiles = collectFiles(root, "src-tauri/src", ".rs");

  for (const relativePath of rustFiles) {
    const source = read(root, relativePath);
    for (const [, target] of withoutRustGuards(source).matchAll(
      /target_os\s*=\s*"([^"]*)"/g,
    )) {
      if (!supportedTargets.has(target)) {
        failures.push(
          `branch for unsupported platform '${target}': ${relativePath}`,
        );
      }
    }
    for (const text of bannedTextHits(
      relativePath,
      source,
      forbiddenRustText,
    )) {
      failures.push(`legacy Rust symbol '${text}' remains: ${relativePath}`);
    }
  }

  if (!hasRustGuard(read(root, "src-tauri/src/lib.rs"))) {
    failures.push("the Rust crate does not reject non-macOS targets");
  }

  const frontendFiles = [
    ...collectFiles(root, "src", ".ts"),
    ...collectFiles(root, "src", ".tsx"),
  ];
  for (const relativePath of frontendFiles) {
    const source = read(root, relativePath);
    for (const text of [
      "typing_tool",
      "external_script_path",
      "useOsType",
      "available_on_platform",
      "data-platform",
    ]) {
      if (source.includes(text)) {
        failures.push(
          `legacy frontend symbol '${text}' remains: ${relativePath}`,
        );
      }
    }
  }

  const packageJson = JSON.parse(read(root, "package.json")) as {
    dependencies: Record<string, string>;
    devDependencies: Record<string, string>;
  };
  for (const dependency of [
    "@tauri-apps/plugin-autostart",
    "@tauri-apps/plugin-clipboard-manager",
    "@tauri-apps/plugin-dialog",
    "@tauri-apps/plugin-fs",
    "@tauri-apps/plugin-global-shortcut",
    "@tauri-apps/plugin-store",
    "react-select",
    "zod",
  ]) {
    if (dependency in packageJson.dependencies) {
      failures.push(`unused frontend dependency remains: ${dependency}`);
    }
  }
  for (const dependency of ["@types/react-select"]) {
    if (dependency in packageJson.devDependencies) {
      failures.push(`unused frontend dependency remains: ${dependency}`);
    }
  }

  for (const relativePath of collectFiles(root, "src/i18n/locales", ".json")) {
    if (read(root, relativePath).includes('"onboardingDescriptionCodex"')) {
      failures.push(`obsolete translation remains: ${relativePath}`);
    }
  }

  const cargoManifest = read(root, "src-tauri/Cargo.toml");
  for (const text of [
    "tauri-plugin-dialog",
    "tauri-plugin-fs",
    "[patch.crates-io]",
    "staticlib",
    "cdylib",
    "murmur_app_lib",
  ]) {
    if (cargoManifest.includes(text)) {
      failures.push(`obsolete Cargo configuration remains: ${text}`);
    }
  }

  for (const [, target] of cargoManifest.matchAll(
    /^\[target\.'cfg\((.*)\)'\.(?:dev-)?dependencies\]$/gm,
  )) {
    if (!allowedCargoTargets.has(target)) {
      failures.push(`Cargo dependencies for an unsupported target: ${target}`);
    }
  }

  failures.push(
    ...capabilityProblems(
      root,
      "src-tauri/capabilities/main.json",
      "main",
      ["main"],
      [
        "default",
        "core:event:allow-listen",
        "core:event:allow-unlisten",
        "core:app:allow-version",
        "os:allow-locale",
        "opener:allow-open-url",
        "opener:allow-default-urls",
        "updater:allow-check",
        "updater:allow-download-and-install",
        "macos-permissions:allow-check-accessibility-permission",
        "macos-permissions:allow-check-microphone-permission",
        "macos-permissions:allow-request-microphone-permission",
      ],
    ),
    ...capabilityProblems(
      root,
      "src-tauri/capabilities/recording-overlay.json",
      "recording-overlay",
      ["recording_overlay"],
      [
        "core:event:allow-listen",
        "core:event:allow-unlisten",
        "core:event:allow-emit-to",
        "os:allow-locale",
        "allow-get-app-settings",
        "allow-cancel-operation",
        "allow-copy-notice-again",
        "allow-copy-meeting-notice",
        "allow-open-meeting-notice",
        "allow-fit-copy-notice",
        "allow-restore-cancelled-dictation",
      ],
    ),
    ...capabilityProblems(
      root,
      "src-tauri/capabilities/call-prompt.json",
      "call-prompt",
      ["call_prompt"],
      [
        "core:event:allow-listen",
        "core:event:allow-unlisten",
        "allow-answer-call-prompt",
        "allow-fit-call-prompt",
        "allow-get-call-prompt",
      ],
    ),
  );

  const commandManifest = read(root, "src-tauri/build.rs")
    .match(/const APP_COMMANDS:[\s\S]*?= &\[([\s\S]*?)\];/)?.[1]
    ?.matchAll(/"([a-z0-9_]+)"/g);
  const invokeManifest = read(root, "src-tauri/src/lib.rs")
    .match(/\.commands\(collect_commands!\[([\s\S]*?)\]\)/)?.[1]
    ?.matchAll(/(?:[a-z_]+::)*([a-z][a-z0-9_]*)\s*,/g);
  const permissionManifest = read(
    root,
    "src-tauri/permissions/default.toml",
  ).matchAll(/"allow-([a-z0-9-]+)"/g);
  const commandSets = [
    commandManifest && [...commandManifest].map((match) => match[1]),
    invokeManifest && [...invokeManifest].map((match) => match[1]),
    [...permissionManifest].map((match) => match[1].replaceAll("-", "_")),
  ];
  if (
    commandSets.some((commands) => !commands) ||
    commandSets.some(
      (commands) => commands && new Set(commands).size !== commands.length,
    ) ||
    commandSets
      .map((commands) => [...(commands ?? [])].sort().join("\n"))
      .some((commands, _index, all) => commands !== all[0])
  ) {
    failures.push(
      "Tauri invoke handler, application ACL manifest, and default command permissions differ",
    );
  }

  const tauriConfig = JSON.parse(read(root, "src-tauri/tauri.conf.json")) as {
    app: { security: { csp: unknown; devCsp: unknown } };
    bundle: { icon: string[]; targets: string[]; externalBin?: string[] };
  };
  // The only bundled executable besides the app: the meetings helper, built
  // by src-tauri/build.rs for each target.
  if (
    (tauriConfig.bundle.externalBin ?? []).join("\n") !==
    "binaries/murmur-meetings-helper"
  ) {
    failures.push("Tauri external binaries differ from the meetings helper");
  }
  const expectedIcons = [
    "icons/32x32.png",
    "icons/128x128.png",
    "icons/128x128@2x.png",
    "icons/icon.icns",
  ];
  if (tauriConfig.bundle.targets.join("\n") !== ["app", "dmg"].join("\n")) {
    failures.push("Tauri bundle targets are not restricted to app and dmg");
  }
  if (tauriConfig.bundle.icon.join("\n") !== expectedIcons.join("\n")) {
    failures.push("Tauri bundle icons are not restricted to the macOS set");
  }

  const expectedCsp =
    "default-src 'self'; base-uri 'self'; form-action 'none'; object-src 'none'; frame-ancestors 'none'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data: asset: http://asset.localhost; media-src 'self' data: asset: http://asset.localhost; font-src 'self' data:; connect-src 'self' ipc: http://ipc.localhost";
  const expectedDevCsp =
    "default-src 'self' http://localhost:1420; base-uri 'self'; form-action 'none'; object-src 'none'; frame-ancestors 'none'; script-src 'self' http://localhost:1420; style-src 'self' 'unsafe-inline' http://localhost:1420; img-src 'self' data: asset: http://asset.localhost; media-src 'self' data: asset: http://asset.localhost; font-src 'self' data:; connect-src 'self' ipc: http://ipc.localhost http://localhost:1420 ws://localhost:1420";
  if (tauriConfig.app.security.csp !== expectedCsp) {
    failures.push("Tauri production CSP differs from the reviewed policy");
  }
  if (tauriConfig.app.security.devCsp !== expectedDevCsp) {
    failures.push("Tauri development CSP differs from the reviewed policy");
  }

  failures.push(...contractProblems(root));

  return {
    failures,
    summary: `${rustFiles.length} Rust files and ${frontendFiles.length} frontend files`,
  };
}

// ---------------------------------------------------------------------------
// Windows
// ---------------------------------------------------------------------------

export const windowsOverlayPath = "src-tauri/tauri.windows.conf.json";

// Paths every Windows tree must hold. Each merge slice that brings Windows
// files adds the ones it brings, in the same commit, so this list always names
// what is on the branch.
export const windowsRequiredPaths: string[] = [
  windowsOverlayPath,
  "src-tauri/windows-app-manifest.xml",
  "src-tauri/icons/icon.ico",
  "src-tauri/src/foreground.rs",
  "src-tauri/src/uia.rs",
  "src-tauri/src/window_placement.rs",
  "src-tauri/src/shortcut/modifier_chord.rs",
  "src-tauri/src/paste_tx/windows.rs",
  "src-tauri/src/input/win32.rs",
  "src-tauri/src/durable/windows.rs",
  "src-tauri/src/mcp/setup.rs",
  "src-tauri/src/meetings/analysis_cpp.rs",
  "src-tauri/src/meetings/analysis_local.rs",
  "src-tauri/src/meetings/call_probe_windows.rs",
  "src-tauri/src/meetings/capture/loopback_logic.rs",
  "src-tauri/src/meetings/capture/wasapi.rs",
  "src-tauri/src/meetings/capture/route.rs",
  "src-tauri/src/meetings/power_windows.rs",
  "src-tauri/src/meetings/archive/aac.rs",
  "src-tauri/src/meetings/archive/media_foundation.rs",
  "src-tauri/src/meetings/archive/resample.rs",
  "src-tauri/src/meetings/trash_windows.rs",
  "src-tauri/src/meetings/diarize/mod.rs",
  "src-tauri/src/meetings/diarize/ahc.rs",
  "src-tauri/src/meetings/diarize/assign.rs",
  "src-tauri/src/meetings/diarize/config.rs",
  "src-tauri/src/meetings/diarize/embedding.rs",
  "src-tauri/src/meetings/diarize/fbank.rs",
  "src-tauri/src/meetings/diarize/parity.rs",
  "src-tauri/src/meetings/diarize/plda.rs",
  "src-tauri/src/meetings/diarize/pyannote.rs",
  "src-tauri/src/meetings/diarize/reconstruct.rs",
  "src-tauri/src/meetings/diarize/segmentation.rs",
  "src-tauri/src/meetings/diarize/types.rs",
  "src-tauri/src/meetings/diarize/vbx.rs",
  "docs/windows-meetings.md",
  "docs/windows-meetings-owner-checks.md",
  "src-tauri/updater-artifacts.conf.json",
  ".github/scripts/minisign.mjs",
  ".github/scripts/windows-updater-manifest.mjs",
  ".github/scripts/windows-updater-manifest.test.mjs",
  ".github/scripts/fixtures/windows-updater/signer.pub",
  ".github/scripts/fixtures/windows-updater/other-signer.pub",
  ".github/scripts/fixtures/windows-updater/Murmur_1.9.0-beta.7_x64-setup.exe",
  ".github/scripts/fixtures/windows-updater/Murmur_1.9.0-beta.7_x64-setup.exe.sig",
  ".github/scripts/check-nextest-timeout.ps1",
  ".github/scripts/windows-media-foundation.ps1",
  ".github/scripts/windows-record-dialogue.ps1",
  ".github/scripts/windows-smoke.ps1",
  ".github/scripts/windows-vc-runtime.ps1",
  ".github/workflows/windows.yml",
  ".github/workflows/meetings-windows.yml",
  ".github/workflows/build-windows.yml",
  ".github/workflows/release-dry-run.yml",
  ".github/scripts/release-dry-run.mjs",
  "src-tauri/updater-test.conf.json",
  "scripts/release-contract.ts",
];

// The Windows app reads the same feed as the Mac app: the overlay holds no
// endpoint, and the base config holds exactly this one. The `windows-beta`
// feed is only a pointer for installs older than the unified release; it may
// appear in docs and release scripts, never in a Tauri config.
export const stableUpdateFeed =
  "https://github.com/DailyXplorer/Murmur/releases/latest/download/latest.json";
export const testUpdateFeed =
  "https://github.com/DailyXplorer/Murmur/releases/download/updater-test/latest.json";

interface UpdaterConfig {
  bundle?: { createUpdaterArtifacts?: boolean };
  plugins?: {
    updater?: {
      pubkey?: string;
      endpoints?: string[];
      windows?: { installMode?: string };
    };
  };
}

/** The Windows updater rules: the shared feed, passive installs, no signing by default. */
export function windowsUpdaterProblems(root: string): string[] {
  const overlay = JSON.parse(read(root, windowsOverlayPath)) as UpdaterConfig;
  const updater = overlay.plugins?.updater;
  const problems: string[] = [];
  if (overlay.bundle?.createUpdaterArtifacts !== false) {
    problems.push(
      "the Windows config builds updater artifacts: a release build turns them on",
    );
  }
  if (updater?.endpoints !== undefined) {
    problems.push(
      "the Windows config overrides the updater endpoints: it reads the shared feed",
    );
  }
  const baseConfig = existsSync(path.join(root, "src-tauri/tauri.conf.json"))
    ? read(root, "src-tauri/tauri.conf.json")
    : "{}";
  const base = JSON.parse(baseConfig) as UpdaterConfig;
  if (
    (base.plugins?.updater?.endpoints ?? []).join("\n") !== stableUpdateFeed
  ) {
    problems.push(
      "tauri.conf.json does not hold exactly the stable update feed as its one endpoint",
    );
  }
  if (updater?.windows?.installMode !== "passive") {
    problems.push("the Windows updater does not install in passive mode");
  }
  if (updater?.pubkey !== undefined) {
    problems.push(
      "the Windows config sets its own updater pubkey: it shares the base one",
    );
  }
  const artifactsPath = "src-tauri/updater-artifacts.conf.json";
  if (existsSync(path.join(root, artifactsPath))) {
    const artifacts = JSON.parse(read(root, artifactsPath)) as UpdaterConfig;
    if (artifacts.bundle?.createUpdaterArtifacts !== true) {
      problems.push(`${artifactsPath} does not turn the artifacts on`);
    }
  }
  const testPath = "src-tauri/updater-test.conf.json";
  if (existsSync(path.join(root, testPath))) {
    const test = JSON.parse(read(root, testPath)) as Record<string, unknown>;
    const { $schema: _schema, ...rest } = test;
    if (
      JSON.stringify(rest) !==
      JSON.stringify({ plugins: { updater: { endpoints: [testUpdateFeed] } } })
    ) {
      problems.push(`${testPath} must hold only the test feed endpoint`);
    }
  }
  for (const name of readdirSync(path.join(root, "src-tauri"))) {
    if (/^tauri.*\.json$/.test(name) || /\.conf\.json$/.test(name)) {
      if (read(root, `src-tauri/${name}`).includes("windows-beta")) {
        problems.push(
          `src-tauri/${name} points at the legacy windows-beta feed`,
        );
      }
    }
  }
  return problems;
}

/**
 * Meetings are on for Windows: nothing hides them there. Each rule fails on
 * the line that used to keep them out.
 */
export function windowsMeetingsProblems(root: string): string[] {
  const problems: string[] = [];
  // A tree with no sources is reported by the required paths, not here.
  if (!existsSync(path.join(root, "src-tauri/src/settings.rs")))
    return problems;
  const rustFunctionBody = (source: string, signature: string): string => {
    const start = source.indexOf(signature);
    if (start === -1) return "";
    const end = source.indexOf("\n}\n", start);
    return source.slice(start, end === -1 ? undefined : end);
  };
  const settingsSource = read(root, "src-tauri/src/settings.rs");
  if (
    rustFunctionBody(settingsSource, "fn default_meetings_enabled()")
      .replace(/\s+/g, " ")
      .trim() !== "fn default_meetings_enabled() -> bool { true"
  ) {
    problems.push("Meetings are not on by default on every platform");
  }
  if (
    rustFunctionBody(settingsSource, "fn restrict_to_windows(").includes(
      "meetings_enabled",
    )
  ) {
    problems.push("restrict_to_windows turns Meetings off on Windows");
  }
  if (!settingsSource.includes("fn turn_on_meetings_once(")) {
    problems.push("a Windows store written with Meetings off is not migrated");
  }
  if (
    /!isWindows[^\n]*meetings_enabled/.test(
      read(root, "src/components/Sidebar.tsx"),
    )
  ) {
    problems.push("the sidebar hides the Meetings page on Windows");
  }
  if (
    /!isWindows[^\n]*<MeetingsToggle/.test(
      read(root, "src/components/settings/advanced/AdvancedSettings.tsx"),
    )
  ) {
    problems.push("the Advanced page hides the Meetings switch on Windows");
  }
  if (
    read(root, "src-tauri/src/commands/meetings.rs").includes(
      "not available on Windows",
    )
  ) {
    problems.push("a meetings command refuses to run on Windows");
  }
  if (!/Meetings/.test(read(root, ".github/scripts/windows-smoke.ps1"))) {
    problems.push("the Windows smoke test does not open the Meetings page");
  }
  return problems;
}

export function checkWindows(
  root: string = defaultRoot,
  required: string[] = windowsRequiredPaths,
): PlatformResult {
  const failures: string[] = [];
  for (const relativePath of required) {
    if (!existsSync(path.join(root, relativePath))) {
      failures.push(`required Windows asset is missing: ${relativePath}`);
    }
  }
  if (required === windowsRequiredPaths) {
    failures.push(...windowsUpdaterProblems(root));
    failures.push(...windowsMeetingsProblems(root));
  }
  return { failures, summary: `${required.length} required paths` };
}

export const PLATFORMS: Platform[] = [
  { name: "macos", enabled: () => true, check: checkMacos },
  {
    name: "windows",
    enabled: (root) => existsSync(path.join(root, windowsOverlayPath)),
    check: (root) => checkWindows(root),
  },
];

export function runPlatforms(root: string = defaultRoot): {
  failures: string[];
  passed: string[];
} {
  const failures: string[] = [];
  const passed: string[] = [];
  for (const platform of PLATFORMS.filter((entry) => entry.enabled(root))) {
    const result = platform.check(root);
    failures.push(
      ...result.failures.map((failure) => `[${platform.name}] ${failure}`),
    );
    if (result.failures.length === 0) {
      passed.push(`${platform.name} (${result.summary})`);
    }
  }
  return { failures, passed };
}

if (import.meta.main) {
  const { failures, passed } = runPlatforms();
  if (failures.length > 0) {
    console.error(failures.map((failure) => `- ${failure}`).join("\n"));
    process.exit(1);
  }
  console.log(`Platform check passed: ${passed.join(", ")}`);
}
