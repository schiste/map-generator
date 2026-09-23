use geo_types::{MultiLineString, MultiPolygon};

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
    /// Code of the enclosing unit (e.g. the région of a département). Borders
    /// between features with different parents are drawn as parent borders.
    pub parent: Option<String>,
    pub geometry: MultiPolygon<f64>,
}

/// A line feature, such as a disputed boundary.
#[derive(Debug, Clone, PartialEq)]
pub struct MapLine {
    pub id: String,
    pub name: String,
    pub geometry: MultiLineString<f64>,
}
