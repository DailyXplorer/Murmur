import { readdirSync, readFileSync } from "node:fs";
import path from "node:path";

export interface VersionSources {
  packageJson: string;
  cargoToml: string;
  cargoLock: string;
  tauriConf: string;
  releaseNoteNames: string[];
}

function readVersionField(label: string, source: string): string | undefined {
  try {
    const version = JSON.parse(source).version;
    return typeof version === "string" && version.length > 0
      ? version
      : undefined;
  } catch {
    throw new Error(`${label} is not valid JSON`);
  }
}

function packageVersion(cargoToml: string): string | undefined {
  const header = cargoToml
    .replaceAll("\r\n", "\n")
    .match(/^\[package\][\s\S]*?(?=^\[)/m);
  return header?.[0].match(/^version = "([^"]+)"/m)?.[1];
}

function lockVersion(cargoLock: string): string | undefined {
  return cargoLock
    .replaceAll("\r\n", "\n")
    .match(/\[\[package\]\]\nname = "murmur"\nversion = "([^"]+)"/)?.[1];
}

/** Problems that mean a release would ship mismatched or undocumented versions. */
export function versionProblems(sources: VersionSources): string[] {
  const problems: string[] = [];
  let npmVersion: string | undefined;
  try {
    npmVersion = readVersionField("package.json", sources.packageJson);
    if (!npmVersion) problems.push("package.json has no version");
  } catch (error) {
    problems.push(
      error instanceof Error ? error.message : "package.json is not valid JSON",
    );
  }

  const cargo = packageVersion(sources.cargoToml);
  if (!cargo) problems.push("src-tauri/Cargo.toml has no package version");

  const locked = lockVersion(sources.cargoLock);
  if (!locked)
    problems.push("src-tauri/Cargo.lock has no murmur package version");

  let tauriVersion: string | undefined;
  try {
    tauriVersion = readVersionField(
      "src-tauri/tauri.conf.json",
      sources.tauriConf,
    );
    if (!tauriVersion)
      problems.push("src-tauri/tauri.conf.json has no version");
  } catch (error) {
    problems.push(
      error instanceof Error
        ? error.message
        : "src-tauri/tauri.conf.json is not valid JSON",
    );
  }

  const found = [
    ["package.json", npmVersion],
    ["src-tauri/Cargo.toml", cargo],
    ["src-tauri/Cargo.lock", locked],
    ["src-tauri/tauri.conf.json", tauriVersion],
  ] as const;
  const versions = new Set(
    found.flatMap(([, version]) => (version ? [version] : [])),
  );
  if (versions.size > 1) {
    problems.push(
      `versions differ: ${found
        .map(([name, version]) => `${name} ${version ?? "missing"}`)
        .join(", ")}`,
    );
  }

  const version = npmVersion ?? cargo ?? tauriVersion;
  if (version && !sources.releaseNoteNames.includes(`${version}.md`)) {
    problems.push(
      `missing release notes src/content/release-notes/${version}.md`,
    );
  }
  return problems;
}

function checkRepository(root: string): string[] {
  const read = (relativePath: string) =>
    readFileSync(path.join(root, relativePath), "utf8");
  const notesDir = path.join(root, "src/content/release-notes");
  return versionProblems({
    packageJson: read("package.json"),
    cargoToml: read("src-tauri/Cargo.toml"),
    cargoLock: read("src-tauri/Cargo.lock"),
    tauriConf: read("src-tauri/tauri.conf.json"),
    releaseNoteNames: readdirSync(notesDir),
  });
}

if (import.meta.main) {
  const problems = checkRepository(path.resolve(import.meta.dir, ".."));
  if (problems.length > 0) {
    console.error(problems.map((problem) => `- ${problem}`).join("\n"));
    process.exit(1);
  }
  console.log("release versions match");
}
