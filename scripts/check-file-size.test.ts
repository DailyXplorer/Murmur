import assert from "node:assert/strict";
import { test } from "bun:test";
import { LIMIT, sizeProblems } from "./check-file-size.ts";

const files = (entries: Record<string, number>) =>
  new Map(Object.entries(entries));

test("files under the limit pass", () => {
  const report = sizeProblems(files({ "a.rs": 10, "b.ts": LIMIT }), {});
  assert.deepEqual(report, { problems: [], notes: [] });
});

test("a new file over the limit fails", () => {
  const { problems } = sizeProblems(files({ "a.rs": LIMIT + 1 }), {});
  assert.equal(problems.length, 1);
  assert.match(problems[0], /^a\.rs: 1001 lines, limit 1000/);
});

test("an allow-listed file may stay at or under its cap", () => {
  const report = sizeProblems(files({ "a.rs": 1500 }), { "a.rs": 1500 });
  assert.deepEqual(report, { problems: [], notes: [] });
});

test("an allow-listed file that grew fails", () => {
  const { problems } = sizeProblems(files({ "a.rs": 1501 }), { "a.rs": 1500 });
  assert.equal(problems.length, 1);
  assert.match(problems[0], /^a\.rs: grew to 1501 lines, its cap is 1500/);
});

test("an allow-listed file that shrank only gets a note", () => {
  const { problems, notes } = sizeProblems(files({ "a.rs": 1200 }), {
    "a.rs": 1500,
  });
  assert.deepEqual(problems, []);
  assert.deepEqual(notes, ["a.rs: 1200 lines, lower its cap from 1500"]);
});

test("an allow-listed file back under the limit only gets a note", () => {
  const { problems, notes } = sizeProblems(files({ "a.rs": 900 }), {
    "a.rs": 1500,
  });
  assert.deepEqual(problems, []);
  assert.deepEqual(notes, ["a.rs: 900 lines, remove it from the allowlist"]);
});

test("an allow-listed file that is gone only gets a note", () => {
  const { problems, notes } = sizeProblems(files({}), { "a.rs": 1500 });
  assert.deepEqual(problems, []);
  assert.deepEqual(notes, ["a.rs: gone, remove it from the allowlist"]);
});

test("a custom limit is honoured", () => {
  const { problems } = sizeProblems(files({ "a.rs": 51 }), {}, 50);
  assert.equal(problems.length, 1);
});
