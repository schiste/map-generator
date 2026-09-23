use std::collections::BTreeMap;

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
    /// Names in other languages, by BCP 47 tag (`fr`, `zh-Hant`), for
    /// multilingual labels (see `RenderOptions::languages`).
    pub names: BTreeMap<String, String>,
    /// CSS class, e.g. `country` or `subdivision`.
    pub class: String,
    /// Code of the enclosing unit (e.g. the région of a département). Borders
    /// between features with different parents are drawn as parent borders.
    pub parent: Option<String>,
    /// Name of the enclosing unit, shown in tooltips ("Lancaster, Nebraska").
    pub parent_name: Option<String>,
    /// Lowercase ISO 3166-1 alpha-2 code of the country the feature belongs
    /// to, emitted as a class so tools like Maphue can colour by country.
    pub country: Option<String>,
    /// Data units the feature belongs to (`data-unit`), see `units`.
    pub units: Vec<String>,
    /// Wikidata item, e.g. `Q142` (`data-wikidata`).
    pub wikidata: Option<String>,
    pub geometry: MultiPolygon<f64>,
}

impl Default for MapFeature {
    fn default() -> Self {
        MapFeature {
            id: String::new(),
            name: String::new(),
            names: BTreeMap::new(),
            class: String::new(),
            parent: None,
            parent_name: None,
            country: None,
            units: Vec::new(),
            wikidata: None,
            geometry: MultiPolygon(vec![]),
        }
    }
}

/// A place drawn as a point, such as a capital.
#[derive(Debug, Clone, PartialEq)]
pub struct MapPlace {
    pub id: String,
    pub name: String,
    /// Names in other languages, as `MapFeature::names`.
    pub names: BTreeMap<String, String>,
    pub kind: PlaceKind,
    /// Lowercase ISO 3166-1 alpha-2 code of its country.
    pub country: Option<String>,
    pub wikidata: Option<String>,
    pub lon: f64,
    pub lat: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum PlaceKind {
    /// A country's capital.
    CountryCapital,
    /// The capital of a state, province or region.
    RegionCapital,
}

/// A line feature, such as a disputed boundary.
#[derive(Debug, Clone, PartialEq)]
pub struct MapLine {
    pub id: String,
    pub name: String,
    pub geometry: MultiLineString<f64>,
}
