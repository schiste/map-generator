#!/usr/bin/env bash
# Regenerates docs/examples from pinned open data (Natural Earth, geoBoundaries,
# US Census). Requires:
#   scripts/fetch-data.sh ne-geojson
#   scripts/fetch-data.sh geoboundaries FRA ADM1
#   scripts/fetch-data.sh geoboundaries USA ADM2
#   scripts/fetch-data.sh us-counties-fips
#   scripts/fetch-data.sh ne-worldview IND
#   scripts/fetch-data.sh ne-worldview PAK
set -euo pipefail

cargo build --release --quiet -p mapgen-cli
M=${MAPGEN:-./target/release/mapgen}
D=data
G="$D/geoboundaries"
E=docs/examples
T=$(mktemp -d)
trap 'rm -rf "$T"' EXIT
CTX=(--context "$D/ne_10m_admin_0.geojson" --lakes "$D/ne_10m_lakes.geojson" --disputed "$D/ne_10m_disputed_lines.geojson")
mkdir -p "$E"

# Départements with régions as parents (thicker borders), overseas départements in insets.
$M render -i "$D/ne_10m_admin_1.geojson" --dataset ne-admin1 --region FRA "${CTX[@]}" \
  --labels --width 900 --title "France — départements" -o "$E/france-departements.svg"
$M render -i "$D/ne_10m_admin_1.geojson" --dataset ne-admin1 --region FRA "${CTX[@]}" \
  --labels --width 900 --title "France — départements" -o "$E/france-departements.html"
$M render -i "$D/ne_10m_admin_0.geojson" --dataset ne-admin0 --continent Europe "${CTX[@]}" \
  --width 900 --title Europe -o "$E/europe.svg"
# Curved label along Chile.
$M render -i "$D/ne_10m_admin_0.geojson" --dataset ne-admin0 --continent "South America" "${CTX[@]}" \
  --labels --width 700 --title "South America" -o "$E/south-america.svg"
$M render -i "$D/ne_10m_admin_0.geojson" --dataset ne-admin0 --frame world \
  --lakes "$D/ne_10m_lakes.geojson" --disputed "$D/ne_10m_disputed_lines.geojson" \
  --theme dark --padding 10 --width 1200 --title World -o "$E/world-dark.svg"
# Labels in Japanese, Korean and Chinese; the viewer's language picks one.
$M render -i "$D/ne_10m_admin_1.geojson" --dataset ne-admin1 --region JPN "${CTX[@]}" \
  --labels --languages ja,ko,zh-Hans,zh-Hant --theme light --width 700 --title Japan \
  -o "$E/japan-light.svg"
# Kashmir from India's and Pakistan's points of view, disputed areas hatched.
for view in IND PAK; do
  $M render -i "$D/ne_10m_admin_0.geojson" --dataset ne-admin0 --worldview "$view" \
    --bbox 66,26,84,38 "${CTX[@]}" --disputed-areas "$D/ne_10m_disputed_areas.geojson" \
    --labels --width 600 --title "Kashmir ($view view)" \
    -o "$E/kashmir-$(echo "$view" | tr '[:upper:]' '[:lower:]').svg"
done
$M render -i "$D/ne_10m_admin_1.geojson" --dataset ne-admin1 --region FJI "${CTX[@]}" \
  --width 500 --title "Fiji (straddles 180°)" -o "$E/fiji.svg"

# geoBoundaries, converted to indexed GeoPackages with readable ids
# (licence read from the .license.json sidecar and credited in the map).
$M convert -i "$G/FRA-ADM1.geojson" --dataset geoboundaries -o "$T/FRA-ADM1.gpkg" \
  --ids-from "$D/ne_10m_admin_1.geojson" --ids-column region_cod
$M render -i "$T/FRA-ADM1.gpkg" --dataset geoboundaries "${CTX[@]}" --labels --credit \
  --width 900 --title "France — régions" -o "$E/france-regions.svg"
$M convert -i "$G/USA-ADM2.geojson" --dataset geoboundaries -o "$T/USA-ADM2.gpkg" \
  --ids-from "$D/us-counties-fips.geojson" --ids-column id --ids-parent-column STATE --ids-prefix US- \
  --parent-names "$D/us-states.txt" --parent-names-key STATE --parent-names-column STATE_NAME
# Albers conic, state borders from the FIPS parents, Alaska/Hawaii/Puerto Rico in insets.
$M render -i "$T/USA-ADM2.gpkg" --dataset geoboundaries "${CTX[@]}" --credit --theme light \
  --simplify 1 --width 1200 --title "United States — counties" -o "$E/usa-counties.svg"
