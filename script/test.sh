#!/usr/bin/env bash
# Wrapper for `swift test`. The vendored WhisperKit hangs debug-symbol
# generation, so the -debug-info-format none flag is mandatory; forgetting it
# makes bare `swift test` hang forever with no diagnostic.
set -euo pipefail
exec swift test -debug-info-format none "$@"
