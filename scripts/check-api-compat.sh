#!/usr/bin/env bash
# The public API stays compatible within a version (Droidtop/tracker#867, owner 2026-11-04: "I should be able to bump
# the version number and immediately get the benefit"). ./gradlew apiCheck (the binary-compatibility validator) fails
# when a module's public API is not its checked-in dump (<module>/api/<module>.api). This adds the version rule: a
# line removed from a dump since the last release is a breaking change, and fails unless lib.version is a new minor
# (0.x) or major version after that release. Adding to the API only needs the new dump.
set -euo pipefail
cd "$(dirname "$0")/.."
last="$(git describe --tags --abbrev=0 --match 'v[0-9]*' HEAD 2>/dev/null || true)"
[ -n "$last" ] || { echo "nothing released yet; nothing to compare"; exit 0; }
removed="$(git diff --unified=0 "$last" -- '*/api/*.api' | grep -E '^-[^-]' || true)"
if [ -z "$removed" ]; then echo "no API removed or changed since $last"; exit 0; fi
now="$(sed -n 's/^lib\.version=//p' gradle.properties)"
IFS=. read -r lmaj lmin _ <<<"${last#v}"; IFS=. read -r nmaj nmin _ <<<"$now"
if [ "$nmaj" -gt "$lmaj" ] || { [ "$lmaj" = 0 ] && [ "$nmaj" = 0 ] && [ "$nmin" -gt "$lmin" ]; }; then
    echo "API changed since $last, and $now is a breaking version; the CHANGELOG must say what broke:"; echo "$removed"; exit 0
fi
echo "API removed or changed since $last, but lib.version $now is not a breaking version after it:" >&2
echo "$removed" >&2
exit 1
