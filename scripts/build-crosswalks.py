#!/usr/bin/env python3
"""Builds the crosswalks the API hosts (`/api/v1/crosswalks`) into
data/crosswalks/: one `<id>.csv` (from,to,weight) and one `<id>.json`
(metadata) each.

us-counties-2010-2020
    US counties, 2010 to 2020 codes (5-digit FIPS), from the Census Bureau's
    2020 county subdivision comparability file (public domain). Weights are
    shares of the 2010 county's land area; overlaps under 1 % (boundary
    realignments rather than real changes) are dropped and the rest
    renormalised. Covers e.g. Valdez-Cordova (02261) splitting into Chugach
    (02063) and Copper River (02066).

us-counties-2020-2022
    Connecticut's eight counties (09001...) to the nine planning regions that
    replaced them as county equivalents in 2022 (09110...), from the Census
    Bureau's 2020 and 2022 Gazetteer county subdivision files (public
    domain): towns kept their codes, so each town ties an old county to a new
    region. Weights are shares of the old county's land area. Every other
    county keeps its code.

Usage: scripts/build-crosswalks.py [data dir]   (default: data)
Python 3.8+, standard library only.
"""

import csv
import io
import json
import sys
import urllib.request
import zipfile
from collections import defaultdict
from pathlib import Path

USER_AGENT = "map-generator (https://github.com/schiste/map-generator)"
COUSUB = (
    "https://www2.census.gov/geo/docs/maps-data/data/rel2020/cousub/"
    "tab20_cousub20_cousub10_natl.txt"
)
GAZETTEER = (
    "https://www2.census.gov/geo/docs/maps-data/data/gazetteer/"
    "{year}_Gazetteer/{year}_Gaz_cousubs_national.zip"
)
MIN_SHARE = 0.01


def fetch(url):
    request = urllib.request.Request(url, headers={"User-Agent": USER_AGENT})
    with urllib.request.urlopen(request, timeout=300) as response:
        return response.read().decode("utf-8-sig")


def fetch_zip_text(url):
    request = urllib.request.Request(url, headers={"User-Agent": USER_AGENT})
    with urllib.request.urlopen(request, timeout=300) as response:
        data = response.read()
    with zipfile.ZipFile(io.BytesIO(data)) as z:
        name = next(n for n in z.namelist() if n.endswith(".txt"))
        return z.read(name).decode("latin-1")


def connecticut_towns(year):
    """Connecticut towns (5-digit county subdivision code) -> (county, land)."""
    rows = csv.reader(io.StringIO(fetch_zip_text(GAZETTEER.format(year=year))), delimiter="\t")
    header = [c.strip() for c in next(rows)]
    towns = {}
    for row in rows:
        r = dict(zip(header, (c.strip() for c in row)))
        if r.get("USPS") == "CT" and not r["GEOID"].endswith("00000"):
            towns[r["GEOID"][5:]] = (r["GEOID"][:5], int(r["ALAND"] or 0))
    return towns


def us_counties_2022(out, counties):
    old, new = connecticut_towns(2020), connecticut_towns(2022)
    overlap = defaultdict(int)
    for town, (county, land) in old.items():
        if town in new:
            overlap[(county, new[town][0])] += land
    by_old = defaultdict(list)
    for (county, region), land in overlap.items():
        by_old[county].append((region, land))
    lines = [(c, c, 1.0) for c in sorted(counties) if not c.startswith("09")]
    for county in sorted(by_old):
        parts = by_old[county]
        total = sum(land for _, land in parts)
        kept = [(r, land) for r, land in parts if land / total >= MIN_SHARE]
        kept_total = sum(land for _, land in kept)
        for region, land in sorted(kept):
            lines.append((county, region, round(land / kept_total, 6)))
    lines.sort()
    write(out, "us-counties-2020-2022", lines, {
        "id": "us-counties-2020-2022",
        "title": "US counties, 2020 to 2022 codes (Connecticut planning regions)",
        "dataset": "us-counties",
        "from": "2020 county FIPS (5 digits)",
        "to": "2022 county FIPS (5 digits)",
        "weighting": "share of the 2020 county's land area, by town; overlaps under 1 % dropped",
        "source": "US Census Bureau, 2020 and 2022 Gazetteer county subdivision files",
        "sourceUrl": GAZETTEER.format(year=2022),
        "licence": "Public domain (US Government work)",
        "codePrefix": "",
    })


def us_counties(out):
    rows = csv.DictReader(io.StringIO(fetch(COUSUB)), delimiter="|")
    overlap = defaultdict(int)
    for r in rows:
        old, new = r["GEOID_COUSUB_10"][:5], r["GEOID_COUSUB_20"][:5]
        if old and new:
            overlap[(old, new)] += int(r["AREALAND_PART"] or 0)
    by_old = defaultdict(list)
    for (old, new), land in overlap.items():
        by_old[old].append((new, land))
    lines = []
    for old in sorted(by_old):
        parts = by_old[old]
        total = sum(land for _, land in parts)
        if total == 0:
            # Water-only counterparts: keep the code as it is.
            lines.append((old, old, 1.0))
            continue
        kept = [(n, land) for n, land in parts if land / total >= MIN_SHARE]
        kept_total = sum(land for _, land in kept)
        for new, land in sorted(kept):
            lines.append((old, new, round(land / kept_total, 6)))
    write(out, "us-counties-2010-2020", lines, {
        "id": "us-counties-2010-2020",
        "title": "US counties, 2010 to 2020 codes",
        "from": "2010 county FIPS (5 digits)",
        "to": "2020 county FIPS (5 digits)",
        "weighting": "share of the 2010 county's land area; overlaps under 1 % dropped",
        "source": "US Census Bureau, 2020 county subdivision comparability file",
        "sourceUrl": COUSUB,
        "licence": "Public domain (US Government work)",
        "codePrefix": "",
        "dataset": "us-counties",
    })
    return {t for _, t, _ in lines}


def write(out, name, lines, meta):
    changed = sorted({f for f, t, _ in lines if f != t})
    meta["rows"] = len(lines)
    meta["changedCodes"] = len(changed)
    with open(out / f"{name}.csv", "w", encoding="utf-8", newline="") as f:
        w = csv.writer(f, lineterminator="\n")
        w.writerow(["from", "to", "weight"])
        w.writerows(lines)
    (out / f"{name}.json").write_text(json.dumps(meta, indent=2) + "\n", encoding="utf-8")
    print(f"{name}: {len(lines)} rows, {len(changed)} code(s) changed", file=sys.stderr)


def main():
    out = Path(sys.argv[1] if len(sys.argv) > 1 else "data") / "crosswalks"
    out.mkdir(parents=True, exist_ok=True)
    counties_2020 = us_counties(out)
    us_counties_2022(out, counties_2020)


if __name__ == "__main__":
    main()
