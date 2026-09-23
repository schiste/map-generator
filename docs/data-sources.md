# Data sources

This project only supports **public-domain or openly licensed** data. Datasets
with non-commercial or no-redistribution terms (such as GADM) are deliberately
not supported.

Downloads go to `./data`, which is git-ignored. Nothing is vendored.

## Natural Earth (public domain)

- Quickest: `scripts/fetch-data.sh ne-geojson` fetches Admin-0, Admin-1 and lakes as GeoJSON (~56 MB).
- Full: `scripts/fetch-data.sh natural-earth` fetches `natural_earth_vector.gpkg` (all themes, all scales).
- Layers used: `ne_10m_admin_0_countries` (filter `ADM0_A3`), `ne_10m_admin_1_states_provinces`
  (filter `adm0_a3`, id `iso_3166_2`), `ne_10m_lakes`.
- Names in about 40 languages: `--name-column name_fr`, `name_de`, `name_ja`…
- Credit: "Natural Earth" (not required, but added automatically).

## geoBoundaries `gbOpen` (open; licence varies by country)

- `scripts/fetch-data.sh geoboundaries FRA ADM2` downloads one country and level; `FRA,DEU` or `ALL`
  downloads several. Add `--simplified` for geoBoundaries' lighter geometry.
- Files: `data/geoboundaries/<ISO3>-<LEVEL>.geojson` plus `<ISO3>-<LEVEL>.license.json` with the
  licence, original source, source URL, the year the boundaries represent and the release
  (boundary id, build date, commit), taken from the geoBoundaries API.
- Boundary version: every map made from such a file says which boundaries it shows, as
  `data-boundary-year` and `data-source-release` on `<svg>` and "boundaries as of <year>" in the
  credit. For sources without this metadata, pass `--boundary-year` (and `--source-release`).
  For example, geoBoundaries' US counties represent 2018, before Connecticut's 2022
  planning regions and Alaska's 2019 Valdez-Cordova split.
- Properties: `shapeName` (name), `shapeISO` (ISO 3166-2, often empty below ADM1), `shapeID`
  (stable id, used as fallback), `shapeGroup` (ISO3).
- Only the `gbOpen` release is used. geoBoundaries' other releases (`gbHumanitarian`,
  `gbAuthoritative`) can carry non-open terms.

Licences seen in practice:

| Example | Licence | Obligation |
| --- | --- | --- |
| USA ADM2 (Census Bureau) | Public domain | None |
| CHN ADM2 | ODC PDDL | None |
| FRA ADM1/ADM2 (IGN) | Etalab Open License 2.0 | Attribution |
| DEU ADM3 (BKG) | Data licence Germany – Attribution 2.0 | Attribution |
| AUT ADM2 (BEV) | CC BY-SA 2.0 | Attribution + share-alike |

`mapgen` embeds the credit in every SVG and warns on share-alike licences. `mapgen batch -i data/geoboundaries`
renders every downloaded file, crediting each map from its own sidecar.
Check the `.license.json` before publishing.

## Natural Earth disputed boundaries (public domain)

`ne_10m_admin_0_boundary_lines_disputed_areas` (fetched by `ne-geojson` as
`data/ne_10m_disputed_lines.geojson`): claim and disputed boundary lines, drawn dashed with
`--disputed`.

## US county FIPS codes (public domain)

`scripts/fetch-data.sh us-counties-fips` fetches US Census county boundaries with their
5-digit FIPS codes as feature ids (as packaged by plotly/datasets, MIT, pinned to a
commit). Use it as a crosswalk reference for geoBoundaries counties:

```sh
mapgen convert -i data/geoboundaries/USA-ADM2.geojson --dataset geoboundaries -o usa.gpkg \
  --ids-from data/us-counties-fips.geojson --ids-column id --ids-parent-column STATE --ids-prefix US-
```

The same command also fetches the Census list of state codes and names
(`data/us-states.txt`, public domain) for `--parent-names`, which puts the state in each
county's tooltip ("Lancaster, Nebraska").

Natural Earth Admin-1 works the same way for ISO 3166-2 codes (`--ids-column iso_3166_2`,
or `region_cod` to get e.g. French régions).

## Planned

- **OpenStreetMap / Geofabrik** (ODbL): fine coastlines and rivers.
- **GeoNames** (CC BY 4.0): label anchor points.
