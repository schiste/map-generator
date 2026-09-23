#!/usr/bin/env bash
# Renders gallery maps with librsvg, the renderer Wikimedia Commons uses for
# SVG thumbnails, and compares them with the reference images in tests/rsvg/.
# Catches output librsvg drops or misplaces (textPath, dominant-baseline, CSS
# it doesn't support) and wrong <switch> language picks.
#
#   scripts/rsvg-check.sh            compare with the references
#   scripts/rsvg-check.sh --update   rewrite the references (look at them!)
#
# Runs in a Debian trixie container (librsvg 2.60, DejaVu and Noto CJK fonts)
# so results don't depend on the host; set RSVG_NATIVE=1 to use the host's
# rsvg-convert and ImageMagick instead.
set -euo pipefail
cd "$(dirname "$0")/.."

if [ -z "${RSVG_NATIVE:-}" ] && [ -z "${RSVG_IN_CONTAINER:-}" ]; then
  exec docker run --rm ${RSVG_PLATFORM:+--platform "$RSVG_PLATFORM"} -e RSVG_IN_CONTAINER=1 -v "$PWD:/w" -w /w debian:trixie bash -c \
    "apt-get update -qq && apt-get install -y -qq --no-install-recommends \
       librsvg2-bin imagemagick fonts-dejavu-core fonts-noto-cjk >/dev/null \
     && scripts/rsvg-check.sh $*"
fi

UPDATE=${1:-}
REF=tests/rsvg
OUT=target/rsvg-check
# Differing pixels (after a 10 % colour fuzz) allowed, per million. Renders
# are identical across arm64 and amd64; a missing short label (Chile at
# 700 px) is ~200 ppm.
TOLERANCE=20
mkdir -p "$OUT" "$REF"
rsvg-convert --version

# name | gallery file | viewer language
CASES=(
  "south-america|south-america.svg|"            # curved label (Chile) as rotated letters
  "france-departements|france-departements.svg|" # leaders, insets, coasts, régions
  "japan-ja|japan-light.svg|ja"                  # <switch>/systemLanguage, CJK widths
  "japan-zh-tw|japan-light.svg|zh-TW"            # region tag picks zh-Hant
  "kashmir-ind|kashmir-ind.svg|"                 # hatch pattern, disputed borders
  "fiji|fiji.svg|"                               # --css-vars: var() with fallbacks
)

failed=0
for case in "${CASES[@]}"; do
  IFS='|' read -r name file lang <<<"$case"
  args=()
  [ -n "$lang" ] && args=(--accept-language="$lang")
  png="$OUT/$name.png"
  if ! warnings=$(rsvg-convert "${args[@]}" "docs/examples/$file" -o "$png" 2>&1) || [ -n "$warnings" ]; then
    echo "FAIL $name: rsvg-convert: $warnings"
    failed=1
    continue
  fi
  if [ "$UPDATE" = "--update" ]; then
    cp "$png" "$REF/$name.png"
    echo "updated $REF/$name.png"
    continue
  fi
  if [ ! -f "$REF/$name.png" ]; then
    echo "FAIL $name: no reference (run with --update)"
    failed=1
    continue
  fi
  read -r w h < <(identify -format '%w %h\n' "$png")
  if [ "$(identify -format '%w %h' "$REF/$name.png")" != "$w $h" ]; then
    echo "FAIL $name: size ${w}x$h differs from the reference"
    failed=1
    continue
  fi
  diff=$(compare -metric AE -fuzz 10% "$REF/$name.png" "$png" "$OUT/$name-diff.png" 2>&1 >/dev/null || true)
  diff=${diff%% *}
  per_million=$(( ${diff%.*} * 1000000 / (w * h) ))
  if [ "$per_million" -gt "$TOLERANCE" ]; then
    echo "FAIL $name: $diff pixels differ ($per_million ppm; diff in $OUT/$name-diff.png)"
    failed=1
  else
    echo "ok   $name ($per_million ppm)"
  fi
done
exit "$failed"
