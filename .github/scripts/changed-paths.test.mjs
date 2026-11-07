/**
 * @fileoverview changed-paths.sh on throw-away repositories: a pull request
 * checked out as a merge commit, and every case that must run everything.
 */
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const script = resolve(
  dirname(fileURLToPath(import.meta.url)),
  "changed-paths.sh",
);
const git = (cwd, ...args) =>
  execFileSync(
    "git",
    [
      "-c",
      "user.name=t",
      "-c",
      "user.email=t@example.invalid",
      "-c",
      "commit.gpgsign=false",
      ...args,
    ],
    { cwd, encoding: "utf8" },
  );
const write = (cwd, file) => {
  mkdirSync(dirname(join(cwd, file)), { recursive: true });
  writeFileSync(join(cwd, file), `${Math.random()}\n`);
};

/** A base commit, a base that moved on, and a PR branch merged into it. */
function pullRequest(changed) {
  const cwd = mkdtempSync(join(tmpdir(), "changed-paths-"));
  git(cwd, "init", "-q", "-b", "main");
  write(cwd, "README.md");
  git(cwd, "add", "-A");
  git(cwd, "commit", "-q", "-m", "base");
  git(cwd, "checkout", "-q", "-b", "pr");
  for (const file of changed) write(cwd, file);
  git(cwd, "add", "-A");
  git(cwd, "commit", "-q", "-m", "pr");
  git(cwd, "checkout", "-q", "main");
  write(cwd, "src-tauri/src/moved-on-in-main.rs");
  git(cwd, "add", "-A");
  git(cwd, "commit", "-q", "-m", "main moves on");
  git(cwd, "merge", "-q", "--no-ff", "-m", "merge", "pr");
  return cwd;
}

const specs = [
  "rust=src-tauri/.*|rust-toolchain\\.toml",
  "frontend=src/.*|tests/.*",
];
const run = (cwd, event) =>
  Object.fromEntries(
    execFileSync(script, specs, {
      cwd,
      encoding: "utf8",
      env: { ...process.env, EVENT_NAME: event, GITHUB_OUTPUT: "" },
      stdio: ["ignore", "pipe", "inherit"],
    })
      .trim()
      .split("\n")
      .map((line) => line.split("=")),
  );
const decide = (changed, event = "pull_request") => {
  const cwd = pullRequest(changed);
  try {
    return run(cwd, event);
  } finally {
    rmSync(cwd, { recursive: true, force: true });
  }
};

// What the base changed since the branch left it is not the pull request's.
assert.deepEqual(decide(["docs/guide.md"]), {
  rust: "false",
  frontend: "false",
});
assert.deepEqual(decide(["src-tauri/src/lib.rs"]), {
  rust: "true",
  frontend: "false",
});
assert.deepEqual(decide(["src/App.tsx", "docs/guide.md"]), {
  rust: "false",
  frontend: "true",
});
assert.deepEqual(decide(["rust-toolchain.toml", "tests/a.spec.ts"]), {
  rust: "true",
  frontend: "true",
});
// The regex covers the whole path: a lookalike directory is not src/.
assert.deepEqual(decide(["docs/src/readme.md", "mysrc-tauri/x.rs"]), {
  rust: "false",
  frontend: "false",
});
// Everything but a pull request runs everything.
for (const event of ["push", "workflow_dispatch", "schedule", ""]) {
  assert.deepEqual(decide(["docs/guide.md"], event), {
    rust: "true",
    frontend: "true",
  });
}
// An unreadable diff (no first parent) runs everything.
{
  const cwd = mkdtempSync(join(tmpdir(), "changed-paths-"));
  try {
    git(cwd, "init", "-q", "-b", "main");
    write(cwd, "README.md");
    git(cwd, "add", "-A");
    git(cwd, "commit", "-q", "-m", "only");
    assert.deepEqual(run(cwd, "pull_request"), {
      rust: "true",
      frontend: "true",
    });
  } finally {
    rmSync(cwd, { recursive: true, force: true });
  }
}
console.log("changed-paths tests passed");
