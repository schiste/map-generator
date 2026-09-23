#!/usr/bin/env bash
# Pre-renders the default map of every region of every hosted dataset, so
# common requests are served from the API's cache. Run after a data update,
# against the running server:
#
#   scripts/warm-cache.sh https://map-generator.toolforge.org/api/v1
#
# Sequential and polite: one request at a time.
set -euo pipefail
base="${1:-http://localhost:8000/api/v1}"
ok=0 failed=0
for dataset in $(curl -fsS "$base/datasets" | python3 -c 'import json,sys; print(" ".join(d["id"] for d in json.load(sys.stdin)))'); do
  for region in $(curl -fsS "$base/datasets/$dataset/regions" | python3 -c 'import json,sys,urllib.parse; print(" ".join(urllib.parse.quote(r["code"]) for r in json.load(sys.stdin)))'); do
    for q in "" "labels=true"; do
      if curl -fsS -o /dev/null "$base/maps/$dataset/$region.svg${q:+?$q}"; then
        ok=$((ok + 1))
      else
        failed=$((failed + 1))
        echo "failed: $dataset/$region ${q}" >&2
      fi
    done
  done
done
echo "warmed $ok map(s), $failed failure(s)"
