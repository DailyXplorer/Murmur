import { spawnSync } from "node:child_process";

export interface CheckStep {
  name: string;
  command: string;
  args: string[];
  cwd?: string;
}

/**
 * Commands the CI `code-quality` job runs, in that order.
 * `format:check` includes `cargo fmt --check`.
 */
export const ciSteps: CheckStep[] = [
  { name: "frontend audit", command: "bun", args: ["audit"] },
  {
    name: "translations",
    command: "bun",
    args: ["run", "check:translations"],
  },
  {
    name: "platform rules",
    command: "bun",
    args: ["run", "check:platforms"],
  },
  {
    name: "platform check tests",
    command: "bun",
    args: ["run", "test:platforms"],
  },
  {
    name: "file size",
    command: "bun",
    args: ["run", "check:file-size"],
  },
  {
    name: "file size tests",
    command: "bun",
    args: ["run", "test:file-size"],
  },
  {
    name: "Playwright path filter",
    command: "bun",
    args: ["run", "check:playwright-paths"],
  },
  {
    name: "Playwright path filter tests",
    command: "bun",
    args: ["run", "test:playwright-paths"],
  },
  {
    name: "no doctest",
    command: "bun",
    args: ["run", "check:doctests"],
  },
  {
    name: "no doctest tests",
    command: "bun",
    args: ["run", "test:doctests"],
  },
  {
    name: "updater manifest",
    command: "bun",
    args: ["run", "test:updater-manifest"],
  },
  {
    name: "release workflow contract",
    command: "bun",
    args: ["run", "test:release-contract"],
  },
  {
    name: "Windows updater feed",
    command: "bun",
    args: ["run", "test:windows-updater"],
  },
  {
    name: "changed paths script",
    command: "bun",
    args: ["run", "test:changed-paths"],
  },
  {
    name: "version check tests",
    command: "bun",
    args: ["run", "test:versions"],
  },
  {
    name: "macOS expanded diff tests",
    command: "bun",
    args: ["run", "test:macos-expanded"],
  },
  {
    name: "release versions",
    command: "bun",
    args: ["run", "check:versions"],
  },
  { name: "eslint", command: "bun", args: ["run", "lint"] },
  { name: "format", command: "bun", args: ["run", "format:check"] },
  { name: "frontend build", command: "bun", args: ["run", "build"] },
  { name: "typecheck tests", command: "bun", args: ["run", "check:types"] },
];

/** Clippy flags used by the macOS `test` workflow. The crate builds on macOS only. */
export function clippyStep(): CheckStep {
  return {
    name: "clippy",
    command: "cargo",
    args: [
      "clippy",
      "--locked",
      "--all-targets",
      "--all-features",
      "--",
      "-D",
      "warnings",
    ],
    cwd: "src-tauri",
  };
}

function runStep(step: CheckStep): number {
  console.log(`\n== ${step.name} ==`);
  const result = spawnSync(step.command, step.args, {
    cwd: step.cwd,
    stdio: "inherit",
  });
  return result.status ?? 1;
}

function main(): void {
  const contract = runStep({
    name: "check contract",
    command: "bun",
    args: ["scripts/check.test.ts"],
  });
  if (contract !== 0) process.exit(contract);

  for (const step of ciSteps) {
    const status = runStep(step);
    if (status !== 0) {
      console.error(`check failed: ${step.name}`);
      process.exit(status);
    }
  }

  if (process.platform === "darwin") {
    const status = runStep(clippyStep());
    if (status !== 0) {
      console.error("check failed: clippy");
      process.exit(status);
    }
  } else {
    console.log(
      "\nclippy skipped: Murmur compiles on macOS only. The macOS test workflow runs cargo clippy --locked --all-targets --all-features -- -D warnings.",
    );
  }

  console.log("\ncheck passed");
}

if (import.meta.main) main();
