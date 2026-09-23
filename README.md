# map-generator

A deterministic pipeline that turns authoritative GIS boundary data into clean,
lightweight, restylable SVG maps of the world, continents, countries, and their
subdivisions. The default style follows Wikimedia Commons location maps.

**Open data only.** Every supported source is public domain or openly licensed,
and each map records its data credit.

![France, départements](docs/examples/france-departements.svg)

```sh
scripts/fetch-data.sh ne-geojson    # Natural Earth, public domain, ~56 MB
cargo build --release
./target/release/mapgen render -i data/ne_10m_admin_1.geojson --dataset ne-admin1 --region FRA \
    --context data/ne_10m_admin_0.geojson --lakes data/ne_10m_lakes.geojson --labels \
    -o france.svg
```

Each region is its own selectable element:

```xml
<path id="FR-75" class="mg-land subdivision" data-name="Paris" d="M…Z"><title>Paris</title></path>
```

## Features

- **Deterministic.** The same input and options give byte-identical output, so maps can be committed, diffed and cached.
- **Equal-area by default.** Region maps use Lambert Azimuthal Equal-Area centred on the region. World maps use Equal Earth.
- **Antimeridian-safe.** Fiji, Russia and Kiribati are centred correctly. World maps are cut along the projection seam.
- **Gap-free simplification.** Shared borders are simplified once (TopoJSON-style arcs), so neighbouring regions never show slivers.
- **Automatic framing.** Frames the main landmass and leaves out far overseas territories (which it reports). `--bbox` and continent presets are available.
- **Layers.** Background, water (sea frame + lakes), neighbouring countries, the mapped regions, and optional labels with collision avoidance.
- **Easy restyling.** Four themes plus a flag for every colour. Colours live in one `<style>` block, can be emitted as CSS custom properties, and `.html` output adds live colour pickers.
- **Fast.** France with context and lakes takes about 0.9 s, mostly parsing 50 MB of GeoJSON (GeoPackages are queried by region instead). `mapgen batch` renders every country in parallel.

## Gallery

| | |
| --- | --- |
| ![Europe](docs/examples/europe.svg) | ![Japan, light theme](docs/examples/japan-light.svg) |
| ![World, dark theme](docs/examples/world-dark.svg) | ![Fiji](docs/examples/fiji.svg) |
| ![France, régions (IGN via geoBoundaries)](docs/examples/france-regions.svg) | ![US counties (Census Bureau, public domain)](docs/examples/usa-counties.svg) |

Open [`docs/examples/france-departements.html`](docs/examples/france-departements.html)
locally for the interactive colour editor. Regenerate everything with `scripts/build-examples.sh`.

## Colours

Every map has five visual layers, each with its own colour:

| Slot | Flag | What it paints |
| --- | --- | --- |
| background | `--background` | Canvas: padding, around a world map's outline, or behind `--water none` |
| water | `--water` | Sea (map frame or globe) and lakes |
| land | `--land` / `--earth` | The regions being mapped |
| context-land | `--context-land` | Neighbouring countries |
| border, context-border, lake-border, label | `--border` … | Strokes and text |

```sh
mapgen render … --theme dark                                   # wikimedia | light | dark | mono
mapgen render … --water '#bfe3f2' --earth '#fff8e7' --background none
mapgen render … -o map.html                                    # interactive colour pickers + "Download SVG"
mapgen render … --css-vars -o map.svg                          # restyle from page CSS (below)
mapgen themes                                                  # list themes and frame presets
```

With `--css-vars`, an SVG inlined in a web page can be recoloured from CSS:

```css
.my-map { --mg-water: #0b3d91; --mg-land: #f5f5f5; --mg-context-land: #ddd; }
.my-map .mg-land:hover { fill: gold; }
#FR-75 { fill: crimson; }
```

## CLI

```sh
# Finer subdivisions from geoBoundaries (ADM1–ADM5, one file per country and level)
scripts/fetch-data.sh geoboundaries ITA ADM2
mapgen render -i data/geoboundaries/ITA-ADM2.geojson --dataset geoboundaries --credit -o italy-provinces.svg

# A continent, a custom box, or the world
mapgen render -i data/ne_10m_admin_0.geojson --dataset ne-admin0 --continent Europe -o europe.svg
mapgen render -i data/ne_10m_admin_0.geojson --dataset ne-admin0 --bbox=-20,25,60,72 -o box.svg
mapgen render -i data/ne_10m_admin_0.geojson --dataset ne-admin0 --frame world --center-lon 150 -o pacific.svg

# Every country's Admin-1 map, in parallel
mapgen batch -i data/ne_10m_admin_1.geojson --dataset ne-admin1 --context data/ne_10m_admin_0.geojson --out-dir out/

# Any GeoJSON or GeoPackage layer
mapgen render -i my.geojson --id-column code --name-column label -o mine.svg
```

Other useful flags include `--width`, `--padding`, `--simplify` (px), `--min-area` (px²), `--margin`,
`--frame auto|all|world`, `--projection auto|laea|equal-earth`, `--name-column name_fr` (Natural Earth ships names in about 40 languages),
and `--attribution` / `--credit` for the data credit.
Run `mapgen render --help` for the full list.

## Architecture

| Crate | Role |
| --- | --- |
| [`mapgen-core`](crates/mapgen-core) | Pure pipeline, no I/O: framing, projection, antimeridian, simplification, clipping, SVG/HTML. |
| [`mapgen-data`](crates/mapgen-data) | Readers for Natural Earth, geoBoundaries, and any GeoPackage or GeoJSON layer. |
| [`mapgen-cli`](crates/mapgen-cli) | The `mapgen` binary. |

```
read (SQL filter) ─► frame ─► pick projection ─► seam split ─► project
   ─► shared-border simplify ─► clip to frame ─► cull specks ─► themed SVG / HTML
```

See [docs/architecture.md](docs/architecture.md).

## Data and licensing

The **code** is MIT-licensed. The project only supports **open data**; each map
carries the credit for its data in a `<desc id="attribution">` element, and `--credit`
also prints it on the map.

| Dataset | Levels | Licence |
| --- | --- | --- |
| [Natural Earth](https://www.naturalearthdata.com/) | Countries, first-level subdivisions, lakes | Public domain |
| [geoBoundaries](https://www.geoboundaries.org/) (`gbOpen` release only) | ADM0–ADM5, per country | Open, varies by country: public domain, CC BY, CC BY-SA, ODbL, national open licences |

geoBoundaries licences come from each country's source, so `scripts/fetch-data.sh`
saves a `.license.json` next to every file. `mapgen` reads it to build the credit and
warns when a licence is share-alike (the resulting maps must then be shared under the
same licence). No dataset is vendored in this repository. See
[docs/data-sources.md](docs/data-sources.md).

## Known limitations and roadmap

- [ ] Borders drawn as a separate mesh, so each border is stroked once. Today each region strokes its own outline, which also draws Natural Earth's 180° cut through Taveuni (Fiji).
- [ ] Rivers and coastlines from OpenStreetMap (ODbL), label points from GeoNames (CC BY)
- [ ] Smarter label placement (pole of inaccessibility, leader lines). Today labels that collide or don't fit are dropped.
- [ ] Insets for overseas territories (today they're reported and left out, or included with `--frame all`)
- [ ] Optional `proj` backend for explicit EPSG codes
- [ ] `mapgen batch` across many geoBoundaries files (today: one input file per run)

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md). Licensed under the [MIT License](LICENSE).
