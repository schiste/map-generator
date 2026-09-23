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

## Stages (`pipeline.rs`)

### 1. Frame (`frame.rs`)

The *anchor* is the geometry whose extent becomes the map frame:

- `auto`: single-linkage clustering of the subject's polygons (bounding boxes within
  500 km, antimeridian-aware), keeping the cluster with the largest area. This drops
  French Guiana and Réunion from France but keeps Corsica.
- `all`: every polygon.
- `bbox`: a densified lon/lat box (presets: `europe`, `oceania`…; `west > east` crosses 180°).
- `world`: no anchor; the frame is the globe outline.

### 2. Projection (`projection.rs`)

- **LAEA** (spherical, authalic radius) centred on the anchor. It preserves area, and
  EPSG:3035 is the same projection centred on 52°N 10°E.
- **Equal Earth** for world maps, or when the anchor spans more than 200° of longitude.

### 3. Antimeridian (`antimeridian.rs`)

The centre longitude comes from `covering_arc`: sort the longitudes, find the widest empty
gap, and take the complement arc. That gives Fiji 178°E, not 0°. LAEA wraps `lon − lon0`
per point, so it has no seam. Equal Earth does, so `split_at_seam` unwraps each ring,
shifts it by 360° as needed, and intersects only crossing polygons with ±180° strips.
Non-crossing polygons keep bit-identical coordinates.

### 4. Simplification (`simplify.rs`)

This is TopoJSON-style: find junctions (vertices whose neighbour pair differs between the
rings using them), cut rings into arcs, deduplicate arcs by canonical orientation,
simplify each arc once with Visvalingam–Whyatt (endpoints fixed), and stitch the rings
back together. The tolerance is given in pixels (`--simplify`) and converted to projected
units² once the scale is known. A region never disappears; if all of it collapses, its
largest part is kept unsimplified.

### 5. Clip and cull

Features are clipped to the frame rectangle (`geo::BooleanOps`). Features entirely inside
the frame skip clipping, so shared borders stay exact. Parts smaller than `--min-area` px²
are dropped, except the largest part of each subject region.

### 6. Output (`svg.rs`, `html.rs`, `theme.rs`)

Layers in paint order: `#background`, `#water`, `#context`, `#land`, `#lakes`, `#labels`.
Every colour is defined once in the `<style>` block (`.mg-water{fill:…}`), optionally as
`var(--mg-water, …)`. Ids are made XML-valid and unique. HTML output inlines the SVG and
adds colour pickers, theme presets, and a download button that bakes the chosen colours
back into a plain SVG.
