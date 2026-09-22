# Architecture

## Principles

1. **Pure core.** `mapgen-core` does no I/O. Everything is `features + options → String`.
2. **Determinism.** No hash-ordered collections in output paths, features are sorted
   by id, and numbers are printed at a fixed precision with trailing zeros stripped
   (`svg::fmt_num`). Golden-file tests in `crates/mapgen-data/tests` enforce this.
3. **Load only what you draw.** Adapters push region filters into SQL and stream rows.

## Stages

### 1. Ingest (`mapgen-data`)

GeoPackage geometries are a small GeoPackage header followed by standard WKB.
`geozero::wkb::GpkgWkb` decodes both. The geometry column is discovered from
`gpkg_geometry_columns` rather than assumed. Table and column names cannot be
SQL parameters, so they are restricted to `[A-Za-z0-9_]` before interpolation.

### 2. Projection (`projection.rs`)

The core always uses a spherical **Lambert Azimuthal Equal-Area** projection
centred on the region, using the authalic radius. This matches the idea behind
EPSG:3035 (which is LAEA centred on 52°N 10°E) and generalises it to every region.
An optional `proj` backend for explicit EPSG codes is on the roadmap.

### 3. Antimeridian (`antimeridian.rs`)

Azimuthal projections have no seam at ±180°. Each point is projected using
`wrap(lon − lon0)`, so a region that straddles the antimeridian stays continuous
**as long as the centre is chosen on the correct side of the globe**. That is
what `center_longitude` is for: it finds the smallest arc that covers all
longitudes. Cylindrical/conic projections for world maps will need true ring
splitting.

### 4. Simplification (`simplify.rs`)

Topology-preserving Visvalingam–Whyatt (`geo::SimplifyVwPreserve`). The
tolerance is specified in **output pixels** and converted to projected units²
once the viewport scale is known. That way the same `--simplify 0.5` means the
same visual fidelity at any map size.

#### Shared-border topology (not yet implemented)

Simplifying each polygon independently can drop different vertices on each
side of a shared border, which leaves slivers and gaps. The planned fix is to
build arcs as TopoJSON does: split rings at junction points, deduplicate the
shared arcs, simplify each arc once, then reassemble the rings.

### 5. SVG (`svg.rs`)

Hand-written serialiser with XML escaping. Each feature becomes one `<path>` with
`id`, `class`, `data-name`, and a `<title>` child for accessibility. Holes use
`fill-rule: evenodd`.
