import assert from "node:assert/strict";
import { doctestProblems } from "./check-doctests.ts";

const problems = (...lines: string[]) =>
  doctestProblems("a.rs", lines.join("\n"));

// Code blocks rustdoc runs or compiles.
for (const info of [
  "",
  "rust",
  "no_run",
  "should_panic",
  "compile_fail",
  "rust,edition2021",
  "ignore-windows",
  "ignore-x86_64",
]) {
  const found = problems(
    "/// Text.",
    "///",
    `/// \`\`\`${info}`,
    "/// let x = 1;",
    "/// ```",
  );
  assert.equal(found.length, 1, info);
}
assert.match(problems("//! ```", "//! f();", "//! ```")[0], /^a\.rs:1: /);
assert.equal(problems("/// ~~~", "/// f();", "/// ~~~").length, 1);

// Blocks it skips.
for (const info of [
  "text",
  "ignore",
  "json",
  "sh",
  "toml",
  "rust,ignore",
  "text,no_run",
]) {
  assert.deepEqual(
    problems(`/// \`\`\`${info}`, "/// f();", "/// ```"),
    [],
    info,
  );
}

// A fence line is not an opener when its info string holds a backtick, and only the same
// character closes: no doctest hides behind either.
assert.equal(
  problems("/// Use ```x``` here.", "/// ```", "/// f();", "/// ```").length,
  1,
);
assert.equal(
  problems(
    "/// ~~~text",
    "/// ```",
    "/// ~~~",
    "/// ```",
    "/// f();",
    "/// ```",
  ).length,
  1,
);

// Doc forms the line scan cannot read are refused; four slashes is an ordinary comment.
assert.equal(problems("/** ```", " * f();", " */").length, 1);
assert.equal(problems('#[doc = "```\nf();\n```"]').length, 1);
assert.deepEqual(problems("//// ```", "//// f();", "//// ```"), []);
assert.deepEqual(problems("#[doc(hidden)]"), []);

// A line after a closed fence does not inherit the blank line before it.
assert.deepEqual(
  problems(
    "/// Text.",
    "///",
    "/// ```text",
    "/// x",
    "/// ```",
    "///     more",
  ),
  [],
);

// The fence closes where it should: a later block is its own.
assert.equal(
  problems("/// ```text", "/// ```", "///", "/// ```", "/// f();", "/// ```")
    .length,
  1,
);
// A longer fence is not closed by a shorter one.
assert.deepEqual(
  problems("/// ````text", "/// ```", "/// f();", "/// ```", "/// ````"),
  [],
);

// Indented blocks run, except inside a list; plain code (not a doc comment) never counts.
assert.equal(problems("/// Text.", "///", "///     f();").length, 1);
assert.equal(problems("/// - one", "///", "///     more of one").length, 0);
assert.equal(
  problems("//! 1. step", "//!    goes on", "//!    still").length,
  0,
);
assert.deepEqual(problems("// ```", 'let s = "```";', "    f();"), []);

console.log("doctest check tests passed");
