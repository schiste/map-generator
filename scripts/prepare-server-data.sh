#!/usr/bin/env bash
# Prepares the data directory the API server reads (MAPGEN_DATA_DIR):
# downloads the pinned open datasets, converts them to indexed GeoPackages,
# builds the hosted crosswalks, and writes datasets.toml.
#
#   scripts/prepare-server-data.sh OUT_DIR [--geoboundaries ADM1] [--worldviews all|IND,PAK]
#
# Needs `mapgen` (MAPGEN, default: target/release/mapgen, built if missing)
# and python3. Downloads go to $DATA_DIR (default: data) and are reused.
# The output directory is written in full and can then replace the live one
# atomically (see docs/toolforge.md).
set -euo pipefail

out="${1:?usage: $0 OUT_DIR [--geoboundaries LEVEL] [--worldviews all|CODES]}"
shift
gb_level=""
views=""
while [[ $# -gt 0 ]]; do
  case "$1" in
    --geoboundaries) gb_level="${2:?level, e.g. ADM1}"; shift ;;
    --worldviews) views="${2:?all or codes, e.g. IND,PAK}"; shift ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
  shift
done

here="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
export DATA_DIR="${DATA_DIR:-$here/data}"
M="${MAPGEN:-$here/target/release/mapgen}"
if [[ ! -x "$M" ]]; then
  (cd "$here" && cargo build --release --quiet -p mapgen-cli)
fi
fetch="$here/scripts/fetch-data.sh"
mkdir -p "$out"

"$fetch" ne-geojson >/dev/null
"$fetch" us-counties-fips >/dev/null
"$fetch" geoboundaries USA ADM2 >/dev/null
all_views=(ARG BDG BRA CHN DEU EGY ESP FRA GBR GRC IDN IND ISO ISR ITA JPN KOR MAR NEP NLD PAK POL PRT PSE RUS SAU SWE TLC TUR TWN UKR USA VNM)
case "$views" in
  "") view_list=() ;;
  all) view_list=("${all_views[@]}") ;;
  *) IFS=',' read -r -a view_list <<<"$views" ;;
esac
for v in "${view_list[@]}"; do "$fetch" ne-worldview "$v" >/dev/null; done
if [[ -n "$gb_level" ]]; then
  "$fetch" geoboundaries ALL "$gb_level" --simplified >/dev/null
fi

convert() { # input output [convert options...]
  local in="$1" dest="$2"; shift 2
  if [[ ! -f "$dest" || "$in" -nt "$dest" ]]; then
    local part="${dest%.gpkg}.part.gpkg"
    "$M" convert -i "$in" -o "$part" "$@" >/dev/null 2>&1
    mv "$part" "$dest"
    # convert copies the input's licence sidecar next to its output.
    if [[ -f "${part%.gpkg}.license.json" ]]; then
      mv "${part%.gpkg}.license.json" "${dest%.gpkg}.license.json"
    fi
  fi
}

convert "$DATA_DIR/ne_10m_admin_0.geojson" "$out/ne_10m_admin_0.gpkg" --dataset ne-admin0
convert "$DATA_DIR/ne_10m_admin_1.geojson" "$out/ne_10m_admin_1.gpkg" --dataset ne-admin1
convert "$DATA_DIR/ne_10m_lakes.geojson" "$out/ne_10m_lakes.gpkg" --table ne_10m_lakes
convert "$DATA_DIR/ne_10m_disputed_areas.geojson" "$out/ne_10m_disputed_areas.gpkg" --table ne_10m_admin_0_disputed_areas
cp "$DATA_DIR/ne_10m_disputed_lines.geojson" "$DATA_DIR/ne_10m_capitals.geojson" "$out/"
for v in "${view_list[@]}"; do
  lv=$(echo "$v" | tr '[:upper:]' '[:lower:]')
  convert "$DATA_DIR/ne_10m_admin_0_$lv.geojson" "$out/ne_10m_admin_0_$lv.gpkg" --dataset ne-admin0
done
# US counties with FIPS codes (US-31109), states as parents and their names,
# as in the gallery.
convert "$DATA_DIR/geoboundaries/USA-ADM2.geojson" "$out/us-counties.gpkg" --dataset geoboundaries \
  --ids-from "$DATA_DIR/us-counties-fips.geojson" --ids-column id --ids-parent-column STATE --ids-prefix US- \
  --parent-names "$DATA_DIR/us-states.txt" --parent-names-key STATE --parent-names-column STATE_NAME
if [[ -n "$gb_level" ]]; then
  mkdir -p "$out/geoboundaries"
  for f in "$DATA_DIR"/geoboundaries/*-"$gb_level".geojson; do
    convert "$f" "$out/geoboundaries/$(basename "${f%.geojson}").gpkg" --dataset geoboundaries
  done
fi
python3 "$here/scripts/build-crosswalks.py" "$out" 2>&1 | sed 's/^/crosswalks: /'

ne_release="natural-earth-vector@$(grep -m1 '^NE_COMMIT=' "$fetch" | cut -d= -f2 | cut -c1-7)"
{
  cat <<TOML
# Written by scripts/prepare-server-data.sh: what mapgen-server serves.

[context]
countries = "ne_10m_admin_0.gpkg"
lakes = "ne_10m_lakes.gpkg"
disputed = "ne_10m_disputed_lines.geojson"
disputed_areas = "ne_10m_disputed_areas.gpkg"
places = "ne_10m_capitals.geojson"

[[dataset]]
id = "ne-admin0"
title = "Countries (Natural Earth 1:10m)"
preset = "ne-admin0"
file = "ne_10m_admin_0.gpkg"
level = "ADM0"
filters = ["CONTINENT"]
world = true
worldviews = $([[ ${#view_list[@]} -gt 0 ]] && echo true || echo false)
languages = true
licence = "Public domain"
licence_url = "https://www.naturalearthdata.com/about/terms-of-use/"
release = "$ne_release"

[[dataset]]
id = "ne-admin1"
title = "States, provinces and other first-level subdivisions (Natural Earth 1:10m)"
preset = "ne-admin1"
file = "ne_10m_admin_1.gpkg"
level = "ADM1"
languages = true
licence = "Public domain"
licence_url = "https://www.naturalearthdata.com/about/terms-of-use/"
release = "$ne_release"

[[dataset]]
id = "us-counties"
title = "US counties with FIPS codes (US-31109), states as parents (geoBoundaries, US Census)"
preset = "geoboundaries"
file = "us-counties.gpkg"
level = "ADM2"
TOML
  if [[ -n "$gb_level" ]]; then
    cat <<TOML

[[dataset]]
id = "geoboundaries-$(echo "$gb_level" | tr '[:upper:]' '[:lower:]')"
title = "geoBoundaries $gb_level (simplified), one file per country"
preset = "geoboundaries"
files = "geoboundaries/{region}-$gb_level.gpkg"
level = "$gb_level"
TOML
  fi
} > "$out/datasets.toml"
echo "prepared $out: $(du -sh "$out" | cut -f1)"
