#!/usr/bin/env bash
# Deploys map-generator to Toolforge (https://map-generator.toolforge.org/).
# Dry run by default: prints what would happen. See docs/toolforge.md.
#
#   scripts/deploy-toolforge.sh [--apply] [--data DIR] [--www] [--build [REF]] [--restart]
#
#   --data DIR   upload a data directory made by scripts/prepare-server-data.sh
#                as a new release, then switch data/current to it (atomic)
#   --www        build the WebAssembly playground and upload it (with its own
#                copy of the Natural Earth files: no third-party requests)
#   --build REF  build the server image from GitHub (default ref: main)
#   --restart    (re)start the webservice with toolforge/service.template
#
# Environment: TOOLFORGE_LOGIN (default schiste), TOOLFORGE_TOOL (default
# map-generator), TOOLFORGE_HOST, TOOLFORGE_SSH_KEY.
set -euo pipefail

mode="dry-run"
data=""
www=0
build=""
restart=0
while [[ $# -gt 0 ]]; do
  case "$1" in
    --dry-run) mode="dry-run" ;;
    --apply) mode="apply" ;;
    --data) data="${2:?--data needs a directory}"; shift ;;
    --www) www=1 ;;
    --build)
      if [[ -n "${2:-}" && "${2:0:2}" != "--" ]]; then build="$2"; shift; else build="main"; fi ;;
    --restart) restart=1 ;;
    --help|-h) sed -n '2,17p' "$0"; exit 0 ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
  shift
done

here="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
login="${TOOLFORGE_LOGIN:-schiste}"
tool="${TOOLFORGE_TOOL:-map-generator}"
host="${TOOLFORGE_HOST:-login.toolforge.org}"
repo="https://github.com/schiste/map-generator.git"
[[ "$login" =~ ^[A-Za-z0-9._@-]+$ ]] || { echo "invalid TOOLFORGE_LOGIN" >&2; exit 2; }
[[ "$tool" =~ ^[a-z0-9][a-z0-9-]*$ ]] || { echo "invalid TOOLFORGE_TOOL" >&2; exit 2; }
[[ -z "$build" || "$build" =~ ^[A-Za-z0-9._/-]+$ ]] || { echo "invalid ref" >&2; exit 2; }
if [[ -n "$data" && ! -f "$data/datasets.toml" ]]; then
  echo "$data has no datasets.toml: make it with scripts/prepare-server-data.sh" >&2
  exit 2
fi

ssh_options=(-o BatchMode=yes)
[[ -n "${TOOLFORGE_SSH_KEY:-}" ]] && ssh_options+=(-i "$TOOLFORGE_SSH_KEY")
remote="${login}@${host}"
project="/data/project/${tool}"
ssh_transport="ssh"
for o in "${ssh_options[@]}"; do printf -v q "%q" "$o"; ssh_transport+=" $q"; done

# Runs a command as the tool (prints it in a dry run).
as_tool() {
  if [[ "$mode" == "apply" ]]; then
    # ssh joins its arguments into one remote command line: quote each.
    ssh "${ssh_options[@]}" "$remote" "become $tool $(printf '%q ' "$@")"
  else
    echo "would run as $tool: $*"
  fi
}

upload() { # source/ destination [rsync options...]
  local src="$1" dest="$2"; shift 2
  local opts=(-az --chmod=Du=rwx,Dgo=rx,Fu=rw,Fgo=r "$@")
  [[ "$mode" == "dry-run" ]] && opts+=(--dry-run --stats)
  rsync "${opts[@]}" --rsync-path="become $tool rsync" -e "$ssh_transport" "$src" "${remote}:${dest}"
}

echo "== $mode: $tool on $host as $login"
as_tool mkdir -p "$project/data/releases" "$project/cache" "$project/www"

if [[ -n "$data" ]]; then
  release="$(date -u +%Y%m%dT%H%M%SZ)"
  echo "== data release $release from $data ($(du -sh "$data" | cut -f1))"
  # Reuse the previous release's files: rsync only sends what changed.
  as_tool sh -c "test -d $project/data/current && cp -al \$(readlink -f $project/data/current)/. $project/data/releases/$release/ || mkdir -p $project/data/releases/$release"
  upload "$data/" "$project/data/releases/$release/" --delete --exclude=cache/
  # Switch atomically, then keep the three newest releases.
  as_tool sh -c "cd $project/data && ln -sfn releases/$release current.new && mv -Tf current.new current && ls -1d releases/* | sort | head -n -3 | xargs -r rm -rf"
fi

if [[ "$www" -eq 1 ]]; then
  echo "== playground"
  stage="$(mktemp -d)"
  trap 'rm -rf "$stage"' EXIT
  (cd "$here/crates/mapgen-wasm" && wasm-pack build --release --target web --out-dir www/pkg >/dev/null)
  cp -R "$here/crates/mapgen-wasm/www/." "$stage/"
  rm -f "$stage/pkg/.gitignore" "$stage/pkg/package.json" "$stage/pkg/README.md"
  # Serve Natural Earth ourselves, under the file names the playground uses.
  mkdir -p "$stage/data"
  d="${DATA_DIR:-$here/data}"
  cp "$d/ne_10m_admin_0.geojson" "$stage/data/ne_10m_admin_0_countries.geojson"
  cp "$d/ne_10m_admin_1.geojson" "$stage/data/ne_10m_admin_1_states_provinces.geojson"
  cp "$d/ne_10m_lakes.geojson" "$stage/data/ne_10m_lakes.geojson"
  cp "$d/ne_10m_disputed_lines.geojson" "$stage/data/ne_10m_admin_0_boundary_lines_disputed_areas.geojson"
  python3 - "$stage/index.html" <<'PY'
import sys
p = sys.argv[1]
html = open(p, encoding="utf-8").read()
tag = '<meta charset="utf-8">'
assert tag in html
open(p, "w", encoding="utf-8").write(html.replace(tag, tag + '\n<meta name="mapgen-data" content="data/">', 1))
PY
  upload "$stage/" "$project/www/" --delete
fi

if [[ -n "$build" ]]; then
  echo "== build $repo@$build"
  commit="$(git -C "$here" rev-parse --short "origin/$build" 2>/dev/null || echo "$build")"
  as_tool toolforge build start "$repo" --ref "$build" --envvar "MAPGEN_COMMIT=$commit"
fi

if [[ "$restart" -eq 1 ]]; then
  echo "== webservice"
  upload "$here/toolforge/service.template" "$project/service.template"
  for kv in "MAPGEN_DATA_DIR=$project/data/current" "MAPGEN_CACHE_DIR=$project/cache" \
            "MAPGEN_WWW_DIR=$project/www" "MAPGEN_MAX_CONCURRENT=2" "MAPGEN_CACHE_MAX_BYTES=3000000000"; do
    as_tool toolforge envvars create "${kv%%=*}" "${kv#*=}" >/dev/null
  done
  as_tool sh -c "cd $project && (toolforge webservice restart || toolforge webservice start)"
fi

if [[ "$mode" == "dry-run" ]]; then
  echo "Dry run complete: add --apply to deploy."
else
  echo "Deployed: https://${tool}.toolforge.org/ (API: /api/v1/)"
fi
