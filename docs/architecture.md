# Architecture

## Principles

1. **Pure core.** `mapgen-core` does no I/O: `render(layers, options) → Rendered`.
2. **Determinism.** Trigonometry goes through the pure-Rust `libm` crate (`math.rs`) rather
   than the platform's math library, whose last bits differ between macOS, glibc, MSVC and
   WebAssembly. With that, output is byte-identical on every OS and in the browser; CI checks
   the gallery on Linux, macOS, Windows and WebAssembly.
   Also, layers are sorted by id, hash maps are used only for lookups (never
   iterated into output), and numbers are printed at a fixed precision with trailing zeros
   stripped (`svg::fmt_num`). Golden tests and `scripts/build-examples.sh` enforce this.
3. **Load only what you draw.** GeoPackage filters run in SQL and rows are streamed.

## Stages

### 1. Frame and insets (`frame.rs`, `pipeline.rs`)

Subject polygons are grouped into *clusters* (single linkage: bounding boxes within
500 km, antimeridian-aware). With `--frame auto`, the heaviest cluster is the main map;
the next ones (up to `--max-insets`, each at least 0.025 % of the main landmass) become
insets: Alaska, Hawaii, the French overseas départements. `--frame all`, `--bbox` and
`--frame world` give a single frame.

Each inset is a separate *panel* with its own projection, box size (a quarter of the
map's short side, scaled by the fourth root of its area relative to the main landmass)
and position: corners first, then along the edges, choosing the box that covers the
least of the main map's land (measured on a 6 px raster) without overlapping another inset.

### 2. Projection (`projection.rs`, `epsg.rs`)

Built-in, spherical, deterministic (`libm`): Lambert azimuthal equal-area (default),
Albers equal-area conic (auto for regions ≥ 45° wide at 20–70° latitude; standard
parallels by the one-sixth rule), Lambert conformal conic, and Equal Earth (world maps).
`epsg:<code>` goes through PROJ behind the `proj` cargo feature.

### 3. Antimeridian (`antimeridian.rs`, `panel.rs`)

The centre longitude comes from `covering_arc` (the complement of the widest empty gap).
When the projection has no seam at ±180°, longitude +180° is rewritten as −180°, so the two
sides of a dataset's cut share vertices; border segments with both ends on ±180° are
dropped in any case (Natural Earth's two sides of Taveuni don't even share vertices).
Equal Earth's seam is handled by `split_at_seam`, whose unwrapping is exact per vertex
(`wrap(lon − lon0)` plus a whole multiple of 360°) so shared borders stay shared.

### 4. Topology, simplification and snapping (`simplify.rs`, `panel.rs`)

`Topology` finds junctions (vertices whose neighbour pair differs between the rings using
them), cuts rings into arcs, stores each arc once, and records which features use it. Arcs
are simplified once with Visvalingam–Whyatt (tolerance in pixels). Neighbouring countries'
vertices within `--snap` pixels of the subject's outline are moved onto it (R-tree nearest
segment), which closes gaps and doubled borders between two datasets.

`Topology::finish` stitches rings back, drops parts under `--min-area` (never a region's
largest part), and returns the border arcs of what survived, each with the features on
either side: one feature used once is *outline*; two features are *internal* or, when their
`parent` codes differ, *parent*; one feature using an arc twice is an internal cut and is
not drawn.

### 5. Clip, borders, labels (`panel.rs`, `labels.rs`)

Geometry and border lines are clipped to the frame (features entirely inside skip it).
Labels are placed in pixels, largest region first: straight at the pole of inaccessibility
(polylabel) or nearby positions, then curved along the principal-axis centreline of long,
thin regions, at 100/85/70 % size; small regions get a leader label outside where the text
covers no region. Collisions use an R-tree.
Each language of `--languages` is placed separately. Curved labels are written as
`textPath` for browsers (`Target::Web`) or as one rotated `<text>` per letter for librsvg,
which has no `textPath` (`Target::Commons`, the default); text is centred by a 0.35 em
baseline shift rather than `dominant-baseline`.

### 6. Output (`svg.rs`, `html.rs`, `theme.rs`)

Main-map layers, bottom to top: `#background`, `#water`, `#context`, `#context-borders`,
`#land`, `#lakes`, `#borders` (one path per kind: internal, parent, coast, external — both also
`mg-border-outline` — or plain outline without a neighbour layer, and disputed),
`#labels`; then one `g.mg-inset` per inset. Fills have no stroke; every colour and width is
in the `<style>` block, optionally as `var(--mg-<slot>, …)`.

## Data tools (`mapgen-data`, `validate.rs`)

- `gpkg_write.rs` writes GeoPackages with the standard R-tree spatial index and B-tree
  indexes on the region and id columns; `read_layer_in` queries them by bounding box.
- `crosswalk.rs` borrows codes from a reference layer: reference features are grouped by
  code and unioned, and a region takes a unit's code when each covers at least half of the
  other.
- `validate.rs` checks for invalid polygons (edge crossings found with an R-tree, since
  `geo`'s `Validation` is quadratic on 100 000-vertex rings), out-of-range coordinates,
  repeated vertices, slivers (Polsby–Popper), overlaps, near-miss borders and duplicate ids.
  `repair` snaps later features onto earlier ones (vertex first, then edge), cleans up, and
  undoes the snapping when it did not reduce near misses.
