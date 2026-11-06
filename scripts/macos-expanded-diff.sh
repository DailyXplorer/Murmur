#!/usr/bin/env bash
# Prints what changed in the macro-expanded macOS crate between two commits.
#
#   scripts/macos-expanded-diff.sh <base> <head>
#
# <base> and <head> are git refs, or directories holding a source tree (for a
# host without .git). Run it on macOS. It builds each side with
# `cargo rustc --lib -- -Zunpretty=expanded` (the `murmur` library holds the
# whole app: src/main.rs only calls into it), normalises the text, and prints
# `diff -u` followed by a summary.
#
# Exit 0: identical. Exit 1: different, read the diff. Exit 2: the tool failed.
#
# MAC_DIFF_WORK   work directory (default ${TMPDIR:-/tmp}/murmur-expanded-diff).
#                 `target/` and `swift-build/` in it are reused between runs;
#                 `out/` holds the last run's expanded-*.rs, expanded.diff, logs.
# MAC_DIFF_KEEP_SRC=1  keep the two source trees after the run.
set -uo pipefail

fail() {
  echo "macos-expanded-diff: $*" >&2
  exit 2
}

[ $# -eq 2 ] || fail "usage: $0 <base> <head>  (git refs or source directories)"
[ "$(uname -s)" = "Darwin" ] || fail "run it on macOS: the crate builds there only"
for tool in cargo bun; do
  command -v "$tool" >/dev/null 2>&1 || fail "$tool is not on PATH"
done

SCRIPT_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
REPO_ROOT=$(cd "$SCRIPT_DIR/.." && pwd)
TOOL="$SCRIPT_DIR/macos-expanded.ts"
WORK=${MAC_DIFF_WORK:-${TMPDIR:-/tmp}/murmur-expanded-diff}
WORK=${WORK%/}
mkdir -p "$WORK" || fail "cannot create $WORK"
WORK=$(cd "$WORK" && pwd -P)

# One run at a time per work directory: the cache is shared.
LOCK="$WORK/.lock"
if ! mkdir "$LOCK" 2>/dev/null; then
  holder=$(cat "$LOCK/pid" 2>/dev/null || true)
  if [ -n "$holder" ] && kill -0 "$holder" 2>/dev/null; then
    fail "run $holder holds $LOCK"
  fi
  rm -rf "$LOCK"
  mkdir "$LOCK" 2>/dev/null || fail "cannot take $LOCK"
fi
echo $$ >"$LOCK/pid"
SRC="$WORK/src-$$"
cleanup() {
  [ "${MAC_DIFF_KEEP_SRC:-}" = 1 ] || rm -rf "$SRC"
  rm -rf "$LOCK"
}
trap cleanup EXIT

OUT="$WORK/out"
rm -rf "$OUT"
mkdir -p "$OUT" "$SRC" "$WORK/target" "$WORK/swift-build"

# prepare <label> <ref-or-dir>: a clean, writable source tree in $SRC/<label>;
# sets LAST_SHA to the commit (or directory) it holds.
# A git ref is exported with `git archive`, which leaves the repository
# untouched; the files get the current time, so cargo never sees them as old.
prepare() {
  local label=$1 ref=$2 tree="$SRC/$1"
  mkdir -p "$tree"
  if [ -d "$ref" ]; then
    (cd "$ref" && tar --exclude=./.git --exclude=./node_modules --exclude=./dist \
      --exclude=./src-tauri/target --exclude=./src-tauri/binaries \
      --exclude=./src-tauri/swift/MurmurMeetingsHelper/.build -cf - .) |
      tar -xf - -C "$tree" || fail "cannot copy $ref"
    LAST_SHA="dir:$(basename "$ref")"
  else
    local sha
    sha=$(git -C "$REPO_ROOT" rev-parse --verify --quiet "$ref^{commit}") ||
      fail "$ref is not a commit of $REPO_ROOT (pass a directory when there is no .git)"
    git -C "$REPO_ROOT" archive "$sha" | tar -xf - -C "$tree" || fail "git archive $ref failed"
    LAST_SHA=${sha:0:12}
  fi
  find "$tree" -type f -exec touch {} + 2>/dev/null
  [ -f "$tree/src-tauri/Cargo.toml" ] || fail "$ref has no src-tauri/Cargo.toml"
  # SwiftPM's checkouts and objects are the slow part of build.rs: keep them.
  rm -rf "$tree/src-tauri/swift/MurmurMeetingsHelper/.build"
  if [ -d "$tree/src-tauri/swift/MurmurMeetingsHelper" ]; then
    ln -s "$WORK/swift-build" "$tree/src-tauri/swift/MurmurMeetingsHelper/.build"
  fi
}

# expand <label> <ref> <sha>: normalised expanded crate in $OUT/expanded-<label>.rs.
expand() {
  local label=$1 tree="$SRC/$1" raw="$OUT/raw-$1.rs"
  echo "expanding $label ($2, $3)..." >&2
  (
    cd "$tree/src-tauri" &&
      RUSTC_BOOTSTRAP=1 CARGO_TARGET_DIR="$WORK/target" \
        cargo rustc --locked --lib --profile dev -- -Zunpretty=expanded
  ) >"$raw" 2>"$OUT/build-$label.log" || {
    tail -n 40 "$OUT/build-$label.log" >&2
    fail "cargo rustc failed for $label (full log: $OUT/build-$label.log)"
  }
  [ -s "$raw" ] || fail "cargo printed no expanded source for $label (log: $OUT/build-$label.log)"
  # src/main.rs is the `murmur` binary: it only calls into the library, and
  # expanding it would build the whole library a second time. It has no macro
  # of its own that matters, so its text goes in as it is.
  {
    printf '\n// ==== src/main.rs (binary, not expanded) ====\n'
    cat "$tree/src-tauri/src/main.rs"
  } >>"$raw"
  bun "$TOOL" normalize \
    --map "$tree=<SRC>" \
    --map "$WORK/target=<TARGET>" \
    --map "$WORK=<WORK>" \
    --map "${CARGO_HOME:-$HOME/.cargo}=<CARGO_HOME>" \
    --map "${RUSTUP_HOME:-$HOME/.rustup}=<RUSTUP_HOME>" \
    --map "$HOME=<HOME>" \
    <"$raw" >"$OUT/expanded-$label.rs" || fail "normalising $label failed"
}

# format: rustfmt makes the diff readable. It runs from $OUT, away from the
# repository's rustfmt.toml, on both sides or neither: one side failing to
# parse must not show up as a difference.
format() {
  local label
  command -v rustfmt >/dev/null 2>&1 || return 1
  for label in base head; do
    # Through stdin: with a file argument rustfmt prints the file name first.
    rustfmt --edition 2021 <"$OUT/expanded-$label.rs" >"$OUT/formatted-$label.rs" 2>"$OUT/rustfmt-$label.log" || return 1
  done
  for label in base head; do
    mv "$OUT/formatted-$label.rs" "$OUT/expanded-$label.rs"
  done
}

# The two builds run one after the other: build.rs runs SwiftPM in the shared
# swift-build directory.
prepare base "$1"
BASE_SHA=$LAST_SHA
prepare head "$2"
HEAD_SHA=$LAST_SHA
expand base "$1" "$BASE_SHA"
expand head "$2" "$HEAD_SHA"

if ! format; then
  echo "note: rustfmt could not parse the expanded text (log: $OUT/rustfmt-*.log); diffing the unformatted text" >&2
fi

BASE_FILE="$OUT/expanded-base.rs"
HEAD_FILE="$OUT/expanded-head.rs"
echo "expanded: base $(wc -l <"$BASE_FILE" | tr -d ' ') lines, head $(wc -l <"$HEAD_FILE" | tr -d ' ') lines (files in $OUT)" >&2

diff -u --label "base $1 ($BASE_SHA)" --label "head $2 ($HEAD_SHA)" \
  "$BASE_FILE" "$HEAD_FILE" >"$OUT/expanded.diff"
status=$?
case $status in
  0)
    echo "identical: the expanded macOS crate is the same for $1 and $2"
    exit 0
    ;;
  1)
    cat "$OUT/expanded.diff"
    echo
    bun "$TOOL" summary "$OUT/expanded.diff" "$BASE_FILE" "$HEAD_FILE" || fail "summary failed"
    exit 1
    ;;
  *) fail "diff failed" ;;
esac
