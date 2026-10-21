// Writes src-tauri/src/local_models/pinned.rs: every file of the local
// models at a pinned Hugging Face commit, with its size and SHA-256.
//
//   bun scripts/generate-model-manifest.ts           rewrite the file
//   bun scripts/generate-model-manifest.ts --check   fail if it is stale
//
// The Core ML file sets copy FluidAudio 0.17.7 (`ModelHub.repoIncludeRule`):
// the compiled bundles each loader needs, plus the `.json` and `.txt` files
// at the repository root. Files kept in git rather than LFS have no SHA-256
// on Hugging Face, so they are fetched, checked against their git blob
// SHA-1, and hashed here.
import { createHash } from "node:crypto";
import { readFileSync, writeFileSync } from "node:fs";
import path from "node:path";

type Source = {
  set: "APPLE_SILICON" | "X86_64";
  repo: string;
  commit: string;
  folder: string;
  /** FluidAudio pins this repository to `commit` and checks a marker. */
  fluidRevisionMarker: boolean;
  /** Bundles (directories) or single files to take. */
  take: string[];
  /** Also take the `.json` and `.txt` files at the repository root. */
  rootMetadata: boolean;
};

const SOURCES: Source[] = [
  {
    set: "APPLE_SILICON",
    repo: "FluidInference/parakeet-tdt-0.6b-v3-coreml",
    commit: "7dd20fe6b1797d35f5e3307e8b1732d9a178edfe",
    folder: "parakeet-tdt-0.6b-v3",
    fluidRevisionMarker: false,
    take: [
      "Preprocessor.mlmodelc",
      "Encoder.mlmodelc",
      "Decoder.mlmodelc",
      "JointDecisionv3.mlmodelc",
    ],
    rootMetadata: true,
  },
  {
    set: "APPLE_SILICON",
    repo: "FluidInference/speaker-diarization-coreml",
    // FluidAudio 0.17.7's `Repo.diarizer.revision`.
    commit: "df2625ac79a7ac6b65ad868fee6d80f320da4232",
    folder: "speaker-diarization",
    fluidRevisionMarker: true,
    take: [
      "Segmentation.mlmodelc",
      "FBank.mlmodelc",
      "Embedding.mlmodelc",
      "PldaRho.mlmodelc",
    ],
    rootMetadata: true,
  },
  {
    set: "X86_64",
    repo: "handy-computer/parakeet-tdt-0.6b-v3-gguf",
    commit: "d8d8c32cb2017c57e5307767343e97870f8a7f53",
    folder: "parakeet-tdt-0.6b-v3",
    fluidRevisionMarker: false,
    take: ["parakeet-tdt-0.6b-v3-Q8_0.gguf"],
    rootMetadata: false,
  },
];

type TreeEntry = {
  type: "file" | "directory";
  oid: string;
  size: number;
  path: string;
  lfs?: { oid: string; size: number };
};

type ManifestFile = { path: string; size: number; sha256: string };

const root = path.resolve(import.meta.dir, "..");
const output = path.join(root, "src-tauri/src/local_models/pinned.rs");

async function tree(source: Source): Promise<TreeEntry[]> {
  const url = `https://huggingface.co/api/models/${source.repo}/tree/${source.commit}?recursive=true`;
  const response = await fetch(url);
  if (!response.ok) throw new Error(`${url}: HTTP ${response.status}`);
  return (await response.json()) as TreeEntry[];
}

function taken(source: Source, file: string): boolean {
  if (source.take.some((item) => file === item || file.startsWith(`${item}/`)))
    return true;
  return (
    source.rootMetadata &&
    !file.includes("/") &&
    (file.endsWith(".json") || file.endsWith(".txt"))
  );
}

async function sha256OfGitFile(
  source: Source,
  entry: TreeEntry,
): Promise<string> {
  const url = `https://huggingface.co/${source.repo}/resolve/${source.commit}/${entry.path}`;
  const response = await fetch(url);
  if (!response.ok) throw new Error(`${url}: HTTP ${response.status}`);
  const bytes = Buffer.from(await response.arrayBuffer());
  const blob = createHash("sha1")
    .update(`blob ${bytes.length}\0`)
    .update(bytes)
    .digest("hex");
  if (bytes.length !== entry.size || blob !== entry.oid) {
    throw new Error(`${source.repo}/${entry.path} does not match its git blob`);
  }
  return createHash("sha256").update(bytes).digest("hex");
}

async function files(source: Source): Promise<ManifestFile[]> {
  const entries = (await tree(source)).filter(
    (entry) => entry.type === "file" && taken(source, entry.path),
  );
  for (const item of source.take) {
    if (
      !entries.some(
        (entry) => entry.path === item || entry.path.startsWith(`${item}/`),
      )
    ) {
      throw new Error(`${source.repo} has no ${item} at ${source.commit}`);
    }
  }
  const result: ManifestFile[] = [];
  for (const entry of entries) {
    const sha256 = entry.lfs
      ? entry.lfs.oid
      : await sha256OfGitFile(source, entry);
    result.push({ path: entry.path, size: entry.size, sha256 });
  }
  return result.sort((a, b) => a.path.localeCompare(b.path));
}

function rustSource(source: Source, list: ManifestFile[]): string {
  const lines = list.map(
    (file) =>
      `            ModelFile {\n                path: ${JSON.stringify(file.path)},\n                size: ${file.size},\n                sha256: "${file.sha256}",\n            },`,
  );
  return [
    "    Source {",
    `        repo: ${JSON.stringify(source.repo)},`,
    `        commit: "${source.commit}",`,
    `        folder: ${JSON.stringify(source.folder)},`,
    `        fluid_revision_marker: ${source.fluidRevisionMarker},`,
    "        files: &[",
    ...lines,
    "        ],",
    "    },",
  ].join("\n");
}

const sets = new Map<string, string[]>();
for (const source of SOURCES) {
  const list = await files(source);
  sets.set(source.set, [
    ...(sets.get(source.set) ?? []),
    rustSource(source, list),
  ]);
}

const unformatted = [
  "// Generated by `bun scripts/generate-model-manifest.ts`. Do not edit.",
  "",
  "use super::manifest::{ModelFile, Source};",
  "",
  ...[...sets].flatMap(([name, sources]) => [
    `pub const ${name}: &[Source] = &[`,
    ...sources,
    "];",
    "",
  ]),
].join("\n");

// As `cargo fmt` leaves it, so the format check and `--check` agree.
const rustfmt = Bun.spawnSync(
  ["rustfmt", "--edition", "2021", "--emit", "stdout"],
  {
    cwd: path.join(root, "src-tauri"),
    stdin: Buffer.from(unformatted),
  },
);
if (rustfmt.exitCode !== 0) {
  throw new Error(`rustfmt failed: ${rustfmt.stderr.toString()}`);
}
const generated = rustfmt.stdout.toString();

if (process.argv.includes("--check")) {
  if (readFileSync(output, "utf8") !== generated) {
    console.error(
      "pinned.rs is stale: run bun scripts/generate-model-manifest.ts",
    );
    process.exit(1);
  }
  console.log("pinned.rs matches the pinned Hugging Face commits");
} else {
  writeFileSync(output, generated);
  console.log(`wrote ${path.relative(root, output)}`);
}
