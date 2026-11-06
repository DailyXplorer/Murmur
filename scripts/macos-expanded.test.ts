import assert from "node:assert/strict";
import {
  formatSummary,
  itemPathAt,
  normalizeExpanded,
  normalizeLocations,
  stripNoise,
  summarizeDiff,
} from "./macos-expanded.ts";

const maps = [
  { from: "/tmp/work/src-1/base", to: "<SRC>" },
  { from: "/tmp/work/target", to: "<TARGET>" },
  { from: "/tmp/work", to: "<WORK>" },
];

// The same source built in two directories gives one text.
const sample = (src: string) =>
  `const OUT: &str = "${src}/src-tauri/icons/icon.png";\n` +
  `const GEN: &str = "/tmp/work/target/debug/build/murmur-1a2b/out/x.rs";\n`;
assert.equal(
  normalizeExpanded(sample("/tmp/work/src-1/base"), maps),
  'const OUT: &str = "<SRC>/src-tauri/icons/icon.png";\n' +
    'const GEN: &str = "<TARGET>/debug/build/murmur-1a2b/out/x.rs";\n',
);

// The longest directory wins, whatever the order of the list.
assert.equal(
  normalizeExpanded("/tmp/work/target/a /tmp/work/b", [...maps].reverse()),
  "<TARGET>/a <WORK>/b",
);

// A trailing slash on a directory does not leave a half-replaced path.
assert.equal(
  normalizeExpanded("/tmp/work/target/a", [
    { from: "/tmp/work/target/", to: "<TARGET>" },
  ]),
  "<TARGET>/a",
);

// Text with no directory in it is untouched.
const plain = "fn main() {\n    let x = 1;\n}\n";
assert.equal(normalizeExpanded(plain, maps), plain);

// The line and column specta writes into a derive move with the source; the
// module path and the rest of the text do not.
assert.equal(
  normalizeLocations(
    [
      'impl_location("src/managers/history.rs:132:48");',
      'impl_location("src/managers/history/mod.rs:30:48");',
      'sid("PaginatedHistory", "::murmur::managers::history:132:48")',
      'let url = "https://example.com:8080:1";',
      'let text = "see src/a.rs for details";',
    ].join("\n"),
  ),
  [
    'impl_location("<location>");',
    'impl_location("<location>");',
    'sid("PaginatedHistory", "::murmur::managers::history:<position>")',
    'let url = "https://example.com:8080:1";',
    'let text = "see src/a.rs for details";',
  ].join("\n"),
);

// Doc comments and blank lines go; code stays.
assert.equal(
  stripNoise(
    [
      "mod a {",
      "    /// Says hello.",
      "    //! inner doc",
      "",
      "    fn hello() {}",
      "    // an ordinary comment stays",
      "    fn bye() {}",
      "}",
    ].join("\n"),
  ),
  [
    "mod a {",
    "    fn hello() {}",
    "    // an ordinary comment stays",
    "    fn bye() {}",
    "}",
  ].join("\n"),
);

// A blank line or a `///` line inside a multi-line string is content, kept.
const prompt = [
  'const P: &str = "first',
  "",
  "/// not a doc comment",
  'last";',
  'const R: &str = r#"raw "quoted"',
  "",
  '"#;',
].join("\n");
assert.equal(stripNoise(prompt), prompt);

// A quote in a char literal and a lifetime do not open a string.
const tricky = [
  "fn f<'a>(x: &'a str) -> char {",
  "    let q = '\"';",
  "    let n = '\\n';",
  "",
  "    q",
  "}",
].join("\n");
assert.equal(stripNoise(tricky), tricky.replace("\n\n", "\n"));

// Text after the string ends is code again.
assert.equal(
  stripNoise('let s = "a\\"b";\n\n/// doc\nlet t = 1;'),
  'let s = "a\\"b";\nlet t = 1;',
);

const crate = [
  "#[prelude_import]",
  "use std::prelude::rust_2021::*;",
  "mod settings {",
  "    pub fn load() {}",
  "    pub fn save() {}",
  "}",
  "pub mod managers {",
  "    pub struct Audio;",
  "    impl Audio {",
  "        fn start(&self) {}",
  "    }",
  "}",
].join("\n");

assert.deepEqual(itemPathAt(crate.split("\n"), 4), ["mod settings", "fn load"]);
assert.deepEqual(itemPathAt(crate.split("\n"), 10), [
  "mod managers",
  "impl Audio",
  "fn start",
]);
assert.deepEqual(itemPathAt(["let x = 1;"], 1), []);

// One edited line in `settings`, one added line in `managers`.
const head = crate
  .replace("pub fn save() {}", "pub fn save() { flush(); }")
  .replace(
    "fn start(&self) {}",
    "fn start(&self) {}\n        fn stop(&self) {}",
  );
const diff = [
  "--- base",
  "+++ head",
  "@@ -4,3 +4,3 @@",
  " pub fn load() {}",
  "-    pub fn save() {}",
  "+    pub fn save() { flush(); }",
  "@@ -10,2 +10,3 @@",
  " fn start(&self) {}",
  "+        fn stop(&self) {}",
  "",
].join("\n");
const summary = summarizeDiff(diff, crate, head);
assert.equal(summary.hunks, 2);
assert.deepEqual(summary.items, { "mod settings": 1, "mod managers": 1 });
assert.deepEqual(summary.sites, [
  { file: "base", line: 5, path: ["mod settings", "fn save"] },
  {
    file: "head",
    line: 11,
    path: ["mod managers", "impl Audio", "fn stop"],
  },
]);
assert.equal(
  formatSummary(summary),
  [
    "summary: 2 hunks",
    "  1  mod managers",
    "  1  mod settings",
    "first changed line of each hunk, in the head file (base file for a removal):",
    "  base:5  mod settings > fn save",
    "  head:11  mod managers > impl Audio > fn stop",
  ].join("\n"),
);

// A hunk that only deletes lines is placed with the base file.
const removed = summarizeDiff(
  "@@ -5,1 +4,0 @@\n-x\n",
  crate,
  "mod only {\n}\n",
);
assert.deepEqual(removed.items, { "mod settings": 1 });

assert.equal(summarizeDiff("", crate, crate).hunks, 0);
assert.equal(
  formatSummary({ hunks: 0, items: {}, sites: [] }),
  "summary: 0 hunks",
);

console.log("macos expanded diff tests passed");
