/**
 * The macOS release path is pinned to what v1.12.1 shipped. `release.yml` and
 * `build.yml` are parsed and compared with a golden file taken from the
 * unedited workflows (`fixtures/release-contract-v1.12.1.json`). The release
 * workflow may add jobs, steps, inputs and `if` guards for Windows and the
 * test channel; it may not change a Mac job, a secret or an environment.
 *
 * `bun scripts/release-contract.ts --emit <release.yml> <build.yml>` prints a
 * golden file. Changing the golden is a deliberate edit, with the reason in the
 * pull request.
 */
import { readFileSync } from "node:fs";
import path from "node:path";

type Json = null | boolean | number | string | Json[] | { [key: string]: Json };
type Obj = { [key: string]: Json };

export interface Golden {
  release: Obj;
  build: Obj;
}

/** The only edits the stable path accepts, each undone before comparing. */
export const channelEdits: ReadonlyArray<readonly [string, string]> = [
  // publish-tauri: the test channel appends the test updater config.
  [
    "${{ matrix.args }}${{ inputs.channel == 'test' && ' --config src-tauri/updater-test.conf.json' || '' }}",
    "${{ matrix.args }}",
  ],
];

const macosJobs = ["publish-tauri"];
const stableJobs = [
  "validate-release-source",
  "create-release",
  "publish-tauri",
  "publish-release",
] as const;

export function parseWorkflow(source: string): Obj {
  return Bun.YAML.parse(source) as Obj;
}

/** Drops the pinned ref of every `uses:` so an action bump does not trip the contract. */
function normalize(value: Json, key = ""): Json {
  if (Array.isArray(value)) return value.map((item) => normalize(item));
  if (value && typeof value === "object") {
    return Object.fromEntries(
      Object.entries(value).map(([name, item]) => [
        name,
        normalize(item, name),
      ]),
    );
  }
  if (key === "uses" && typeof value === "string" && value.includes("@")) {
    return value.slice(0, value.indexOf("@"));
  }
  return value;
}

function undoChannelEdits(value: Json): Json {
  if (typeof value === "string") {
    return channelEdits.reduce(
      (text, [edited, original]) => text.replaceAll(edited, original),
      value,
    );
  }
  if (Array.isArray(value)) return value.map(undoChannelEdits);
  if (value && typeof value === "object") {
    return Object.fromEntries(
      Object.entries(value).map(([key, item]) => [key, undoChannelEdits(item)]),
    );
  }
  return value;
}

export function extractGolden(releaseYml: string, buildYml: string): Golden {
  const release = normalize(parseWorkflow(releaseYml)) as Obj;
  const jobs = release.jobs as Obj;
  return {
    release: {
      on: release.on,
      jobs: Object.fromEntries(stableJobs.map((id) => [id, jobs[id]])),
    },
    build: normalize(parseWorkflow(buildYml)) as Obj,
  };
}

const same = (left: Json | undefined, right: Json | undefined) =>
  JSON.stringify(left) === JSON.stringify(right);

function asObj(value: Json | undefined): Obj {
  return value && typeof value === "object" && !Array.isArray(value)
    ? value
    : {};
}

function asList(value: Json | undefined): Json[] {
  return Array.isArray(value) ? value : [];
}

function needsOf(job: Obj): string[] {
  const needs = job.needs;
  if (typeof needs === "string") return [needs];
  return asList(needs).map(String);
}

/** Every golden step is still there, in order, as it was (a new step may sit between). */
function stepProblems(id: string, golden: Json[], current: Json[]): string[] {
  const problems: string[] = [];
  let from = 0;
  for (const original of golden) {
    const name = asObj(original).name;
    const index = current.findIndex(
      (step, position) => position >= from && asObj(step).name === name,
    );
    if (index === -1) {
      problems.push(`${id}: step "${name}" is gone or moved`);
      continue;
    }
    from = index + 1;
    const step = asObj(undoChannelEdits(current[index]));
    const before = asObj(original);
    // A guard or an env var may be added; nothing else about the step may move.
    const { if: guard, env, ...rest } = step;
    const { if: originalGuard, env: originalEnv, ...originalRest } = before;
    if (!same(rest, originalRest)) {
      problems.push(`${id}: step "${name}" changed`);
    }
    if (originalGuard !== undefined && !same(guard, originalGuard)) {
      problems.push(`${id}: step "${name}" lost its condition`);
    }
    for (const [variable, value] of Object.entries(asObj(originalEnv))) {
      if (!same(asObj(env)[variable], value)) {
        problems.push(`${id}: step "${name}" env ${variable} changed`);
      }
    }
  }
  return problems;
}

/**
 * What differs between the workflows of this tree and the golden macOS path.
 * An empty list means the stable release is the v1.12.1 release.
 */
export function releaseContractProblems(
  releaseYml: string,
  buildYml: string,
  golden: Golden,
): string[] {
  const problems: string[] = [];
  const release = normalize(parseWorkflow(releaseYml)) as Obj;
  const jobs = asObj(release.jobs);
  const goldenJobs = asObj(golden.release.jobs);

  if (!same(normalize(parseWorkflow(buildYml)), golden.build)) {
    problems.push("build.yml differs from the pinned macOS build workflow");
  }

  const trigger = asObj(release.on).workflow_dispatch;
  if (trigger === undefined) {
    problems.push("release.yml is no longer started by workflow_dispatch");
  }

  for (const id of stableJobs) {
    const original = asObj(goldenJobs[id]);
    const job = asObj(jobs[id]);
    if (Object.keys(job).length === 0) {
      problems.push(`job ${id} is gone`);
      continue;
    }
    const wanted = needsOf(original);
    const needs = needsOf(job);
    const missing = wanted.filter((need) => !needs.includes(need));
    if (missing.length > 0) {
      problems.push(`${id} no longer needs ${missing.join(", ")}`);
    }
    if (id !== "publish-release" && !same(needs, wanted)) {
      problems.push(
        `${id} needs ${needs.join(", ")}, not ${wanted.join(", ")}`,
      );
    }
    for (const [key, value] of Object.entries(original)) {
      if (key === "needs" || key === "steps") continue;
      if (!same(undoChannelEdits(job[key]), value)) {
        problems.push(`${id}: ${key} changed`);
      }
    }
    const added = Object.keys(job).filter(
      (key) => !(key in original) && key !== "steps",
    );
    const allowed = id === "publish-release" ? ["if", "env"] : [];
    for (const key of added.filter((key) => !allowed.includes(key))) {
      problems.push(`${id}: new key ${key}`);
    }
    problems.push(
      ...stepProblems(id, asList(original.steps), asList(job.steps)),
    );
  }

  // No Windows secret, job or installer in the macOS jobs.
  for (const id of macosJobs) {
    const text = JSON.stringify(jobs[id]);
    if (/windows|\.exe|nsis/i.test(text)) {
      problems.push(`${id} mentions Windows`);
    }
  }
  const stray = Object.entries(jobs).filter(
    ([id, job]) =>
      !(stableJobs as readonly string[]).includes(id) &&
      id !== "publish-windows" &&
      !asObj(job).uses,
  );
  for (const [id] of stray) problems.push(`unexpected job ${id}`);

  problems.push(...windowsJobProblems(jobs));
  problems.push(...channelProblems(jobs));
  return problems;
}

function windowsJobProblems(jobs: Obj): string[] {
  const problems: string[] = [];
  const windows = asObj(jobs["publish-windows"]);
  if (Object.keys(windows).length === 0) {
    return ["job publish-windows is missing"];
  }
  if (windows.uses !== "./.github/workflows/build-windows.yml") {
    problems.push("publish-windows does not call build-windows.yml");
  }
  if (
    windows.if !== "${{ inputs.windows }}" &&
    windows.if !== "inputs.windows"
  ) {
    problems.push("publish-windows does not run only when inputs.windows");
  }
  const secrets = Object.keys(asObj(windows.secrets)).sort();
  if (
    !same(secrets, [
      "TAURI_SIGNING_PRIVATE_KEY",
      "TAURI_SIGNING_PRIVATE_KEY_PASSWORD",
    ])
  ) {
    problems.push(
      `publish-windows passes ${secrets.join(", ")}: only the two updater signing secrets`,
    );
  }
  const needs = needsOf(windows);
  for (const need of ["validate-release-source", "create-release"]) {
    if (!needs.includes(need))
      problems.push(`publish-windows does not need ${need}`);
  }
  const publish = asObj(jobs["publish-release"]);
  if (!needsOf(publish).includes("publish-windows")) {
    problems.push("publish-release does not wait for publish-windows");
  }
  const guard = String(publish.if ?? "");
  if (
    !/publish-windows\.result == 'success'/.test(guard) ||
    !/publish-windows\.result == 'skipped'/.test(guard)
  ) {
    problems.push(
      "publish-release must accept publish-windows only as success, or skipped when windows is off",
    );
  }
  return problems;
}

/** The stable feed and Latest are written by stable steps only. */
function channelProblems(jobs: Obj): string[] {
  const problems: string[] = [];
  const steps = asList(asObj(jobs["publish-release"]).steps).map(asObj);
  for (const step of steps) {
    const text = JSON.stringify(step);
    const stableOnly =
      text.includes("make_latest: 'true'") ||
      text.includes("releases/latest/download/latest.json") ||
      text.includes("windows-beta");
    if (
      stableOnly &&
      !String(step.if ?? "").includes("inputs.channel == 'stable'")
    ) {
      problems.push(
        `publish-release: step "${step.name}" touches the stable feed or Latest without inputs.channel == 'stable'`,
      );
    }
    if (
      text.includes("make_latest: 'true'") &&
      text.includes("prerelease: true")
    ) {
      problems.push(
        `publish-release: step "${step.name}" is both Latest and a prerelease`,
      );
    }
    if (
      String(step.if ?? "").includes("inputs.channel == 'test'") &&
      text.includes("make_latest: 'true'")
    ) {
      problems.push(
        `publish-release: test step "${step.name}" makes the release Latest`,
      );
    }
  }
  if (
    !steps.some(
      (step) =>
        String(step.if ?? "").includes("inputs.channel == 'test'") &&
        JSON.stringify(step).includes("prerelease: true"),
    )
  ) {
    problems.push(
      "publish-release has no test-channel step that keeps the release a prerelease",
    );
  }
  return problems;
}

export const goldenPath = "scripts/fixtures/release-contract-v1.12.1.json";

export function checkRepository(root: string): string[] {
  const read = (relative: string) =>
    readFileSync(path.join(root, relative), "utf8");
  const golden = JSON.parse(read(goldenPath)) as Golden;
  return releaseContractProblems(
    read(".github/workflows/release.yml"),
    read(".github/workflows/build.yml"),
    golden,
  );
}

if (import.meta.main) {
  const [flag, releasePath, buildPath] = process.argv.slice(2);
  if (flag === "--emit") {
    const golden = extractGolden(
      readFileSync(releasePath, "utf8"),
      readFileSync(buildPath, "utf8"),
    );
    console.log(JSON.stringify(golden, null, 2));
  } else {
    const problems = checkRepository(path.resolve(import.meta.dir, ".."));
    if (problems.length > 0) {
      console.error(problems.map((problem) => `- ${problem}`).join("\n"));
      process.exit(1);
    }
    console.log("release workflows match the macOS contract");
  }
}
