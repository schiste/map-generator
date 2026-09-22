# map-generator

A deterministic pipeline that turns authoritative GIS boundary data into clean,
lightweight SVG maps of continents, countries, and their subdivisions, styled
like Wikimedia Commons location maps.

```sh
mapgen render --gpkg data/gadm_410-levels.gpkg --level 1 --region FRA -o france-regions.svg
```

```xml
<path id="FRA.11_1" class="subdivision" data-name="Île-de-France" d="M…Z"><title>Île-de-France</title></path>
```

> **Status: early scaffold.** The end-to-end path works (GeoPackage/GeoJSON →
> projection → simplification → SVG), but see the [roadmap](#roadmap) for
> what's still missing.

## Why

- **Deterministic.** The same input and options always give byte-identical SVG,
  so generated maps can be committed, diffed, and cached.
- **Lightweight.** Region filtering happens in SQL against the on-disk
  GeoPackage, so a job only ever loads the region it draws.
- **Correct by default.** Every map uses an equal-area projection centred on
  its own region, so the choice doesn't depend on a hand-maintained EPSG table.

## Pipeline

```
GeoPackage / GeoJSON ──► filter (SQL WHERE) ──► pick LAEA centre ──► project
      ──► Visvalingam–Whyatt simplify (tolerance in pixels) ──► SVG writer
```

| Crate | Role |
| --- | --- |
| [`mapgen-core`](crates/mapgen-core) | Pure geometry pipeline: projection, antimeridian, simplification, SVG. No I/O. |
| [`mapgen-data`](crates/mapgen-data) | Readers for GADM / Natural Earth GeoPackages and GeoJSON. |
| [`mapgen-cli`](crates/mapgen-cli) | The `mapgen` binary. |

See [docs/architecture.md](docs/architecture.md) for design decisions.

## Getting started

```sh
cargo build --release
scripts/fetch-data.sh natural-earth       # public domain, ~400 MB
./target/release/mapgen render --gpkg data/natural_earth_vector.gpkg \
    --dataset ne-admin1 --region FRA -o france.svg

# No download needed: render the bundled test fixture
./target/release/mapgen render \
    --geojson crates/mapgen-data/tests/fixtures/twin-regions.geojson -o twin.svg
```

## Data and licensing

The **code** is MIT-licensed. **Maps you generate carry the licence of the data
they were built from**:

| Dataset | Licence | Notes |
| --- | --- | --- |
| [Natural Earth](https://www.naturalearthdata.com/) | Public domain | Safe for any use. |
| [GADM 4.1](https://gadm.org/license.html) | Free for non-commercial use; redistribution not allowed without permission | Do not publish GADM-derived maps commercially. |
| [OpenStreetMap](https://www.openstreetmap.org/copyright) (planned) | ODbL | Attribution + share-alike. |
| [GeoNames](https://www.geonames.org/) (planned) | CC BY 4.0 | Attribution. |

No dataset is vendored in this repository. See [docs/data-sources.md](docs/data-sources.md).

## Roadmap

- [ ] Antimeridian-aware projection centre (Fiji, Russia, Kiribati), see `center_longitude`
- [ ] Shared-border topology: simplify each shared arc once (TopoJSON-style) so neighbours never gap
- [ ] ISO 3166-2 ids for GADM subdivisions (GADM `GID_*` → ISO crosswalk)
- [ ] Context layer: neighbouring countries and water, in muted colours
- [ ] `mapgen batch`: render every country / Admin-1 in parallel (rayon)
- [ ] Continent / world maps with cylindrical or pseudo-cylindrical projections and seam splitting
- [ ] Optional `proj` feature for explicit EPSG codes
- [ ] Labels from GeoNames

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md). Licensed under the [MIT License](LICENSE).
