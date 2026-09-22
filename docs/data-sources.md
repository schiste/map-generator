# Data sources

`scripts/fetch-data.sh` downloads datasets into `./data`, which is git-ignored.

## Natural Earth (public domain)

- File: `natural_earth_vector.gpkg` (all themes, all scales)
- Layers used: `ne_10m_admin_0_countries` (filter `ADM0_A3`),
  `ne_10m_admin_1_states_provinces` (filter `adm0_a3`, id `iso_3166_2`)
- Good for world and continent views, and for Admin-1 maps that already have ISO 3166-2 ids.

## GADM 4.1 (non-commercial)

- File: `gadm_410-levels.gpkg`, with layers `ADM_0` … `ADM_5`
- Filter column `GID_0` (ISO 3166-1 alpha-3), id `GID_n`, name `NAME_n`
- **Licence:** free for academic and other non-commercial use. Redistribution
  or commercial use requires permission from GADM. Maps generated from GADM
  inherit this restriction regardless of this project's MIT licence.

## Planned

- **OpenStreetMap / Geofabrik** (ODbL): fine coastlines, rivers.
- **GeoNames** (CC BY 4.0): multilingual names, label anchor points.
