#!/usr/bin/env bash
# Downloads open datasets into ./data (git-ignored). Only public-domain or
# openly licensed sources are supported; see docs/data-sources.md.
#
#   scripts/fetch-data.sh ne-geojson                 # Natural Earth, ~56 MB, public domain
#   scripts/fetch-data.sh natural-earth              # Natural Earth GeoPackage, ~400 MB
#   scripts/fetch-data.sh geoboundaries FRA ADM2     # geoBoundaries gbOpen, one country/level
#   scripts/fetch-data.sh geoboundaries FRA,DEU ADM1 --simplified
#   scripts/fetch-data.sh geoboundaries ALL ADM1     # every country (large)
#   scripts/fetch-data.sh us-counties-fips           # Census county codes (for convert --ids-from)
#   scripts/fetch-data.sh ne-worldview IND           # Natural Earth's Indian view of borders (--worldview)
#
# geoBoundaries licences vary by country (public domain, CC BY, CC BY-SA,
# ODbL, national open licences...). Each download gets a `.license.json`
# sidecar that mapgen reads to credit the source in every map.
set -euo pipefail

DATA_DIR="${DATA_DIR:-data}"
PY="$(command -v python3 || command -v python)"
# Natural Earth GeoJSON, pinned to a commit so downloads (and docs/examples) are reproducible.
NE_COMMIT=ca96624a56bd078437bca8184e78163e5039ad19
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
    base="https://raw.githubusercontent.com/nvkelso/natural-earth-vector/$NE_COMMIT/geojson"
    fetch "$base/ne_10m_admin_0_countries.geojson" "$DATA_DIR/ne_10m_admin_0.geojson"
    fetch "$base/ne_10m_admin_1_states_provinces.geojson" "$DATA_DIR/ne_10m_admin_1.geojson"
    fetch "$base/ne_10m_lakes.geojson" "$DATA_DIR/ne_10m_lakes.geojson"
    fetch "$base/ne_10m_admin_0_boundary_lines_disputed_areas.geojson" "$DATA_DIR/ne_10m_disputed_lines.geojson"
    fetch "$base/ne_10m_admin_0_disputed_areas.geojson" "$DATA_DIR/ne_10m_disputed_areas.geojson"
    # Capitals only (--capitals), with their names, country and Wikidata item.
    if [[ ! -f "$DATA_DIR/ne_10m_capitals.geojson" ]]; then
      fetch "$base/ne_10m_populated_places.geojson" "$DATA_DIR/ne_10m_populated_places.geojson"
      "$PY" - "$DATA_DIR/ne_10m_populated_places.geojson" "$DATA_DIR/ne_10m_capitals.geojson" <<'PY'
import json, sys

src, dest = sys.argv[1:3]
CAPITALS = {"Admin-0 capital", "Admin-0 capital alt", "Admin-1 capital",
            "Admin-1 region capital", "Admin-0 region capital"}
KEEP = ("FEATURECLA", "NAME", "ADM0_A3", "ADM1NAME", "WIKIDATAID", "NE_ID")
features = []
for f in json.load(open(src, encoding="utf-8"))["features"]:
    p = f["properties"]
    if p.get("FEATURECLA") not in CAPITALS:
        continue
    props = {k: p[k] for k in KEEP if p.get(k) not in (None, "")}
    props.update({k: v for k, v in p.items() if k.startswith("NAME_") and v})
    lon, lat = f["geometry"]["coordinates"][:2]
    features.append({"type": "Feature", "properties": props,
                     "geometry": {"type": "Point", "coordinates": [round(lon, 5), round(lat, 5)]}})
features.sort(key=lambda f: f["properties"]["NE_ID"])
with open(dest, "w", encoding="utf-8") as out:
    json.dump({"type": "FeatureCollection", "features": features}, out,
              ensure_ascii=False, separators=(",", ":"))
print(f"{dest}: {len(features)} capitals")
PY
      rm -f "$DATA_DIR/ne_10m_populated_places.geojson"
    fi
    ls -1 "$DATA_DIR"/*.geojson
    ;;
  natural-earth)
    fetch "https://naciscdn.org/naturalearth/packages/natural_earth_vector.gpkg.zip" \
      "$DATA_DIR/natural_earth_vector.gpkg.zip"
    unzip -n -d "$DATA_DIR" "$DATA_DIR/natural_earth_vector.gpkg.zip"
    # Archives may nest the .gpkg in a subfolder; flatten so paths are predictable.
    find "$DATA_DIR" -mindepth 2 -name '*.gpkg' -exec mv -n {} "$DATA_DIR/" \;
    ls -1 "$DATA_DIR"/*.gpkg
    ;;
  geoboundaries)
    isos="${2:?usage: $0 geoboundaries ISO3[,ISO3...]|ALL ADM0..ADM5 [--simplified]}"
    level="${3:?usage: $0 geoboundaries ISO3[,ISO3...]|ALL ADM0..ADM5 [--simplified]}"
    out="$DATA_DIR/geoboundaries"
    mkdir -p "$out"
    "$PY" - "$out" "$isos" "$level" "${4:-}" <<'PY'
import json, os, sys, urllib.request

out, isos, level, variant = sys.argv[1:5]
api = "https://www.geoboundaries.org/api/current/gbOpen/{}/{}/"

def get(url):
    with urllib.request.urlopen(url, timeout=120) as r:
        return r.read()

codes = ["ALL"] if isos.upper() == "ALL" else [c.strip().upper() for c in isos.split(",")]
metas = []
for code in codes:
    data = json.loads(get(api.format(code, level.upper())))
    metas.extend(data if isinstance(data, list) else [data])

for m in metas:
    iso, lvl = m["boundaryISO"], m["boundaryType"]
    url = m["simplifiedGeometryGeoJSON"] if variant == "--simplified" else m["gjDownloadURL"]
    dest = os.path.join(out, f"{iso}-{lvl}.geojson")
    if not os.path.exists(dest):
        tmp = dest + ".part"
        with open(tmp, "wb") as f:
            f.write(get(url))
        os.replace(tmp, dest)
    sidecar = {
        "license": m["boundaryLicense"],
        "source": m["boundarySource"],
        "via": "geoBoundaries",
        "source_url": m.get("boundarySourceURL"),
        "license_url": m.get("licenseSource"),
        "year": m.get("boundaryYearRepresented"),
        # Which release of the boundaries: id, build date and repository commit.
        "release": "{}, built {}, geoBoundaries@{}".format(
            m["boundaryID"], m.get("buildDate"), url.split("/raw/")[1].split("/")[0] if "/raw/" in url else "?"),
        "iso": iso,
        "level": lvl,
    }
    # Explicit UTF-8 and \n: Windows would otherwise write cp1252 and \r\n.
    with open(os.path.join(out, f"{iso}-{lvl}.license.json"), "w", encoding="utf-8", newline="\n") as f:
        json.dump(sidecar, f, indent=2, ensure_ascii=False)
        f.write("\n")
    print(f"{dest}  [{m['boundaryLicense']}; {m['boundarySource']}]")
PY
    ;;
  ne-worldview)
    # Natural Earth point-of-view variants of the country layers (--worldview).
    view="${2:?usage: $0 ne-worldview CODE (e.g. IND, PAK, CHN, UKR, RUS, ISO)}"
    view=$(echo "$view" | tr '[:upper:]' '[:lower:]')
    base="https://raw.githubusercontent.com/nvkelso/natural-earth-vector/$NE_COMMIT/geojson"
    fetch "$base/ne_10m_admin_0_countries_$view.geojson" "$DATA_DIR/ne_10m_admin_0_$view.geojson"
    ;;
  us-counties-fips)
    # US Census county boundaries with 5-digit FIPS codes as feature ids
    # (public-domain Census data, as packaged by plotly/datasets, MIT), pinned.
    fetch "https://raw.githubusercontent.com/plotly/datasets/0c447c47b757ad74edecab31f0d72f849d2e67c2/geojson-counties-fips.json" \
      "$DATA_DIR/us-counties-fips.geojson"
    # State FIPS codes and names (Census Bureau, public domain), for tooltips.
    fetch "https://www2.census.gov/geo/docs/reference/state.txt" "$DATA_DIR/us-states.txt"
    ;;
  *)
    echo "usage: $0 {ne-geojson|ne-worldview CODE|natural-earth|geoboundaries ISO3 LEVEL|us-counties-fips}" >&2
    exit 1
    ;;
esac
