#!/usr/bin/env bash
# Launches a built Murmur.app on its own and fails unless it finishes setup.
#
#   scripts/smoke-macos-app.sh <path to Murmur.app>
#
# The app runs with a throwaway HOME, so it starts as on a first launch: no
# settings, no onboarding done, nothing asks for a permission. It passes when
# the log gets its "Setup complete" line, no panic record shows up while the
# app is watched for a few more seconds, and a SIGTERM stops it. On a failure
# it prints the tail of murmur.log and of the app's own output.
#
# SMOKE_TIMEOUT  seconds to wait for the line (default 15)
# SMOKE_SETTLE   seconds to keep watching the app after the line (default 3)
#
# Run it on a CI runner. The single-instance lock lives in /tmp and the login
# item belongs to the bundle identifier, so on a Mac that has Murmur installed
# it refuses to start while Murmur runs, and it switches Open at Login off.
# Test a copy of the bundle with another CFBundleIdentifier there.
set -euo pipefail

timeout=${SMOKE_TIMEOUT:-15}
settle=${SMOKE_SETTLE:-3}
app=${1:-}
home=""
pid=""

fail() {
  echo "smoke: $1" >&2
  if [ -n "$home" ]; then
    log=$(find "$home/Library/Logs" -name murmur.log 2>/dev/null | head -n 1 || true)
    if [ -n "$log" ]; then
      if grep -q 'panicked at' "$log"; then
        # The record starts with the location and the load address, then
        # comes a backtrace that can be longer than any fixed tail.
        echo "--- murmur.log from the panic record" >&2
        sed -n '/panicked at/,$p' "$log" | head -n 100 >&2 || true
      else
        echo "--- last lines of murmur.log" >&2
        tail -n 60 "$log" >&2
      fi
    fi
    echo "--- last lines of the app's output" >&2
    tail -n 60 "$home/output.log" >&2
  fi
  exit 1
}

cleanup() {
  if [ -n "$pid" ] && kill -0 "$pid" 2>/dev/null; then
    { kill -KILL "$pid" && wait "$pid"; } 2>/dev/null || true
  fi
  if [ -n "$home" ]; then
    rm -rf "$home"
  fi
}
trap cleanup EXIT

now_ms() {
  perl -MTime::HiRes=time -e 'printf "%d\n", time * 1000'
}

alive() {
  kill -0 "$pid" 2>/dev/null
}

panic_logged() {
  [ -n "$log" ] && grep -q 'panicked at' "$log"
}

if [ -z "$app" ]; then
  echo "usage: scripts/smoke-macos-app.sh <path to Murmur.app>" >&2
  exit 2
fi
[ -f "$app/Contents/Info.plist" ] || fail "'$app' is not an app bundle"
executable=$(plutil -extract CFBundleExecutable raw -o - "$app/Contents/Info.plist")
[ -x "$app/Contents/MacOS/$executable" ] || fail "$app/Contents/MacOS/$executable is not executable"
if pgrep -x "$executable" >/dev/null; then
  fail "another $executable is running, and its single-instance lock would take this launch"
fi

# Under /tmp, not $TMPDIR: a unix socket path may not pass 104 bytes on macOS.
# CFFIXED_USER_HOME points the Cocoa side (WebKit storage, preferences) at the
# same throwaway home as HOME does for the Rust side.
home=$(mktemp -d /tmp/murmur-smoke.XXXXXX)
log=""

HOME="$home" CFFIXED_USER_HOME="$home" TMPDIR="$home" \
  "$app/Contents/MacOS/$executable" --start-hidden >"$home/output.log" 2>&1 &
pid=$!
started=$(now_ms)

while :; do
  log=$(find "$home/Library/Logs" -name murmur.log 2>/dev/null | head -n 1 || true)
  if panic_logged; then
    fail "a panic was logged during setup"
  fi
  if [ -n "$log" ] && grep -q 'Setup complete' "$log"; then
    break
  fi
  if ! alive; then
    status=0
    wait "$pid" || status=$?
    pid=""
    fail "the app exited with status $status before 'Setup complete'"
  fi
  if [ $(($(now_ms) - started)) -ge $((timeout * 1000)) ]; then
    fail "no 'Setup complete' line within ${timeout}s"
  fi
  sleep 0.1
done
setup_ms=$(($(now_ms) - started))
setup_line=$(grep -m 1 'Setup complete' "$log")

settled=$(($(now_ms) + settle * 1000))
while [ "$(now_ms)" -lt "$settled" ]; do
  if panic_logged; then
    fail "a panic was logged after setup"
  fi
  if ! alive; then
    status=0
    wait "$pid" || status=$?
    pid=""
    fail "the app exited with status $status after setup"
  fi
  sleep 0.1
done

kill -TERM "$pid"
# bash reports a job killed by a signal on stderr; that line is noise here.
stopped=no
{
  for _ in $(seq 1 100); do
    if ! alive; then
      stopped=yes
      break
    fi
    sleep 0.1
  done
  if [ "$stopped" = yes ]; then
    wait "$pid" || true
  fi
} 2>/dev/null
[ "$stopped" = yes ] || fail "the app ignored SIGTERM for 10s"
pid=""

echo "smoke: ok, 'Setup complete' after ${setup_ms} ms"
echo "smoke: $setup_line"
