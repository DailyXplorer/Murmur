import { existsSync, readFileSync, readdirSync } from "node:fs";
import path from "node:path";

const root = path.resolve(import.meta.dir, "..");
const failures: string[] = [];

const obsoletePaths = [
  ".cargo/config.toml",
  "src-tauri/tauri.windows.conf.json",
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
];

const requiredPaths = [
  "src-tauri/icons/icon.png",
  "src-tauri/icons/tray/tray.svg",
  "src-tauri/resources/tray.png",
];

function collectFiles(directory: string, extension: string): string[] {
  const absoluteDirectory = path.join(root, directory);
  return readdirSync(absoluteDirectory, { withFileTypes: true }).flatMap(
    (entry) => {
      const relative = path.join(directory, entry.name);
      if (entry.isDirectory()) return collectFiles(relative, extension);
      return entry.name.endsWith(extension) ? [relative] : [];
    },
  );
}

function read(relativePath: string): string {
  return readFileSync(path.join(root, relativePath), "utf8");
}

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

const rustGuard = [
  '#[cfg(not(target_os = "macos"))]',
  'compile_error!("Murmur supports macOS only.");',
].join("\n");
const rustFiles = collectFiles("src-tauri/src", ".rs");
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
];

for (const relativePath of rustFiles) {
  const source = read(relativePath);
  const sourceWithoutGuard = source.replace(rustGuard, "");
  if (sourceWithoutGuard.includes("target_os")) {
    failures.push(`platform branch remains: ${relativePath}`);
  }
  for (const text of forbiddenRustText) {
    if (source.includes(text)) {
      failures.push(`legacy Rust symbol '${text}' remains: ${relativePath}`);
    }
  }
}

if (!read("src-tauri/src/lib.rs").includes(rustGuard)) {
  failures.push("the Rust crate does not reject non-macOS targets");
}

const frontendFiles = [
  ...collectFiles("src", ".ts"),
  ...collectFiles("src", ".tsx"),
];
for (const relativePath of frontendFiles) {
  const source = read(relativePath);
  for (const text of ["typing_tool", "external_script_path", "useOsType"]) {
    if (source.includes(text)) {
      failures.push(
        `legacy frontend symbol '${text}' remains: ${relativePath}`,
      );
    }
  }
}

const packageJson = JSON.parse(read("package.json")) as {
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
if ("@types/react-select" in packageJson.devDependencies) {
  failures.push("unused frontend dependency remains: @types/react-select");
}

const cargoManifest = read("src-tauri/Cargo.toml");
for (const text of [
  "tauri-plugin-dialog",
  "tauri-plugin-fs",
  "[patch.crates-io]",
  "[target.'cfg(",
]) {
  if (cargoManifest.includes(text)) {
    failures.push(`obsolete Cargo configuration remains: ${text}`);
  }
}

const capability = JSON.parse(read("src-tauri/capabilities/default.json")) as {
  permissions: unknown[];
};
const expectedPermissions = [
  "core:default",
  "macos-permissions:default",
  "opener:default",
  "process:default",
  "updater:default",
];
const actualPermissions = capability.permissions
  .filter((permission): permission is string => typeof permission === "string")
  .sort();
if (
  capability.permissions.length !== expectedPermissions.length ||
  actualPermissions.join("\n") !== expectedPermissions.sort().join("\n")
) {
  failures.push("frontend capabilities exceed the macOS UI plugin set");
}

const tauriConfig = JSON.parse(read("src-tauri/tauri.conf.json")) as {
  bundle: { icon: string[]; targets: string[] };
};
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

if (failures.length > 0) {
  console.error(failures.map((failure) => `- ${failure}`).join("\n"));
  process.exit(1);
}

console.log(
  `macOS-only check passed for ${rustFiles.length} Rust files and ${frontendFiles.length} frontend files`,
);
