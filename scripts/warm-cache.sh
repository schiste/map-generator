#!/usr/bin/env bash
# Pre-renders the default map of every region of every hosted dataset, so
# common requests are served from the API's cache. Run after a data update,
# against the running server:
#
#   scripts/warm-cache.sh https://map-generator.toolforge.org/api/v1
#
# Sequential and polite: one request at a time. CURL overrides the curl
# binary (default: curl on PATH). SKIP lists datasets to leave out (default
# `mixed`: its maps are combinations, and its single regions repeat others).
set -euo pipefail
base="${1:-http://localhost:8000/api/v1}"
curl="${CURL:-curl}"
json_list() { # url python-expression-over-d
  local body
  body="$("$curl" -fsS "$1")"
  python3 -c "import json,sys,urllib.parse; d=json.loads(sys.argv[1]); print(' '.join($2))" "$body"
}
datasets="$(json_list "$base/datasets" 'x["id"] for x in d')"
[[ -n "$datasets" ]] || { echo "no datasets at $base" >&2; exit 1; }
ok=0 failed=0
skip=" ${SKIP-mixed} "
for dataset in $datasets; do
  [[ "$skip" == *" $dataset "* ]] && continue
  regions="$(json_list "$base/datasets/$dataset/regions" 'urllib.parse.quote(x["code"]) for x in d')"
  for region in $regions; do
    for q in "" "labels=true"; do
      if "$curl" -fsS -o /dev/null "$base/maps/$dataset/$region.svg${q:+?$q}"; then
        ok=$((ok + 1))
      else
        failed=$((failed + 1))
        echo "failed: $dataset/$region ${q}" >&2
      fi
    done
  done
  echo "$dataset: done ($ok ok, $failed failed so far)"
done
echo "warmed $ok map(s), $failed failure(s)"
[[ "$failed" -eq 0 ]]
