#!/usr/bin/env bash
# Downloads the source datasets into ./data (git-ignored).
#
#   scripts/fetch-data.sh ne-geojson      # ~56 MB, public domain (quickest start)
#   scripts/fetch-data.sh natural-earth   # ~400 MB GeoPackage, public domain
#   scripts/fetch-data.sh gadm            # ~2.6 GB, non-commercial licence
#
# Read docs/data-sources.md before redistributing anything built from GADM.
set -euo pipefail

DATA_DIR="${DATA_DIR:-data}"
mkdir -p "$DATA_DIR"

fetch() {
  local url="$1" dest="$2"
  if [[ -f "$dest" ]]; then
    echo "already present: $dest"
    return
  fi
  echo "downloading $url"
  curl -fL --retry 3 -o "$dest.part" "$url"
  mv "$dest.part" "$dest"
}

case "${1:-}" in
  ne-geojson)
    base=https://raw.githubusercontent.com/nvkelso/natural-earth-vector/master/geojson
    fetch "$base/ne_10m_admin_0_countries.geojson" "$DATA_DIR/ne_10m_admin_0.geojson"
    fetch "$base/ne_10m_admin_1_states_provinces.geojson" "$DATA_DIR/ne_10m_admin_1.geojson"
    fetch "$base/ne_10m_lakes.geojson" "$DATA_DIR/ne_10m_lakes.geojson"
    ls -1 "$DATA_DIR"/*.geojson
    exit 0
    ;;
  natural-earth)
    fetch "https://naciscdn.org/naturalearth/packages/natural_earth_vector.gpkg.zip" \
      "$DATA_DIR/natural_earth_vector.gpkg.zip"
    unzip -n -d "$DATA_DIR" "$DATA_DIR/natural_earth_vector.gpkg.zip"
    ;;
  gadm)
    fetch "https://geodata.ucdavis.edu/gadm/gadm4.1/gadm_410-levels.zip" \
      "$DATA_DIR/gadm_410-levels.zip"
    unzip -n -d "$DATA_DIR" "$DATA_DIR/gadm_410-levels.zip"
    ;;
  *)
    echo "usage: $0 {ne-geojson|natural-earth|gadm}" >&2
    exit 1
    ;;
esac

# Archives may nest the .gpkg in a subfolder; flatten so paths are predictable.
find "$DATA_DIR" -mindepth 2 -name '*.gpkg' -exec mv -n {} "$DATA_DIR/" \;
ls -1 "$DATA_DIR"/*.gpkg
