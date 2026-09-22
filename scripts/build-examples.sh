#!/usr/bin/env bash
# Regenerates docs/examples from Natural Earth (public domain).
# Requires: scripts/fetch-data.sh ne-geojson
set -euo pipefail

cargo build --release --quiet
M=./target/release/mapgen
D=data
E=docs/examples
CTX=(--context "$D/ne_10m_admin_0.geojson" --lakes "$D/ne_10m_lakes.geojson")
mkdir -p "$E"

$M render -i "$D/ne_10m_admin_1.geojson" --dataset ne-admin1 --region FRA "${CTX[@]}" \
  --labels --width 900 --title "France — départements" -o "$E/france-departements.svg"
$M render -i "$D/ne_10m_admin_1.geojson" --dataset ne-admin1 --region FRA "${CTX[@]}" \
  --labels --width 900 --title "France — départements" -o "$E/france-departements.html"
$M render -i "$D/ne_10m_admin_0.geojson" --dataset ne-admin0 --continent Europe "${CTX[@]}" \
  --width 900 --title Europe -o "$E/europe.svg"
$M render -i "$D/ne_10m_admin_0.geojson" --dataset ne-admin0 --frame world \
  --lakes "$D/ne_10m_lakes.geojson" --theme dark --padding 10 --width 1200 --title World \
  -o "$E/world-dark.svg"
$M render -i "$D/ne_10m_admin_1.geojson" --dataset ne-admin1 --region JPN "${CTX[@]}" \
  --theme light --width 700 --title Japan -o "$E/japan-light.svg"
$M render -i "$D/ne_10m_admin_1.geojson" --dataset ne-admin1 --region FJI "${CTX[@]}" \
  --width 500 --title "Fiji (straddles 180°)" -o "$E/fiji.svg"
