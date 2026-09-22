# Data sources

`scripts/fetch-data.sh` downloads datasets into `./data`, which is git-ignored.

## Natural Earth (public domain)

- Quickest: `scripts/fetch-data.sh ne-geojson` gets Admin-0, Admin-1 and lakes as GeoJSON (~56 MB).
- Full: `natural_earth_vector.gpkg` (all themes, all scales)
- Layers used: `ne_10m_admin_0_countries` (filter `ADM0_A3`),
  `ne_10m_admin_1_states_provinces` (filter `adm0_a3`, id `iso_3166_2`)
- Lakes: `ne_10m_lakes` (use with `--lakes`); countries as context: Admin-0 (use with `--context`).
- Names in ~40 languages: `--name-column name_fr`, `name_de`, `name_ja`…
- Good for world and continent views, and for Admin-1 maps that already have ISO 3166-2 ids.

## GADM 4.1 (non-commercial)

- File: `gadm_410-levels.gpkg`, with layers `ADM_0` … `ADM_5`
- Filter column `GID_0` (ISO 3166-1 alpha-3), id `GID_n` (level 1 prefers `ISO_1`, the ISO 3166-2 code), name `NAME_n`
- **Licence:** free for academic and other non-commercial use. Redistribution
  or commercial use requires permission from GADM. Maps generated from GADM
  inherit this restriction regardless of this project's MIT licence.

## Planned

- **OpenStreetMap / Geofabrik** (ODbL): fine coastlines, rivers.
- **GeoNames** (CC BY 4.0): label anchor points.
