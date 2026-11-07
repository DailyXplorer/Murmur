#!/usr/bin/env bash
# Tells a workflow which of its jobs a pull request needs.
#
#   changed-paths.sh NAME=REGEX [NAME=REGEX ...]
#
# Writes NAME=true or NAME=false to $GITHUB_OUTPUT (stdout when unset) for each
# argument. A name is true when a file the pull request changes matches its
# extended regular expression, matched against the whole path.
#
# Every doubt runs the job: anything but a pull_request event (push to main,
# workflow_dispatch, schedule), a diff that cannot be read and a diff with no
# file at all make every name true. The checkout must hold the merge commit
# and its first parent (`fetch-depth: 2`): a pull request is checked out as
# the merge of its head into the base, so `HEAD^1..HEAD` is what the pull
# request changes, whatever the base has moved on to.
set -euo pipefail

emit() {
  if [[ -n "${GITHUB_OUTPUT:-}" ]]; then echo "$1" >>"$GITHUB_OUTPUT"; else echo "$1"; fi
}

run_all=true
files=""

if [[ "${EVENT_NAME:-}" == "pull_request" ]]; then
  if files=$(git diff --name-only HEAD^1 HEAD 2>/dev/null) && [[ -n "$files" ]]; then
    run_all=false
  else
    echo "changed-paths: no readable diff, running everything" >&2
  fi
fi

for spec in "$@"; do
  name=${spec%%=*}
  regex=${spec#*=}
  if [[ "$run_all" == true ]] || grep -E -q -x "$regex" <<<"$files"; then
    emit "$name=true"
  else
    emit "$name=false"
  fi
done
