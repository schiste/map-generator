use geo_types::MultiPolygon;

/// A single administrative area ready to be rendered.
///
/// Coordinates are WGS84 longitude/latitude in degrees until the pipeline
/// projects them.
#[derive(Debug, Clone, PartialEq)]
pub struct MapFeature {
    /// Stable identifier emitted as the SVG `id`, e.g. an ISO 3166-2 code (`FR-IDF`).
    pub id: String,
    /// Human-readable name, emitted as `data-name` and a `<title>` child.
    pub name: String,
    /// CSS class, e.g. `country` or `subdivision`.
    pub class: String,
    pub geometry: MultiPolygon<f64>,
}
