use std::collections::BTreeMap;
use std::path::Path;

use geo_types::Geometry;
use mapgen_core::{MapFeature, MapLine};
use serde_json::Value;

use crate::error::Result;
use crate::geometry::{into_multilinestring, into_multipolygon};
use crate::layer::{meaningful, LayerQuery};

/// One GeoJSON feature with the properties a [`LayerQuery`] asks for.
struct RawFeature {
    id: String,
    name: Option<String>,
    names: BTreeMap<String, String>,
    filter: Option<String>,
    parent: Option<String>,
    parent_name: Option<String>,
    country: Option<String>,
    wikidata: Option<String>,
    geometry: Geometry<f64>,
}

/// Reads a GeoJSON `FeatureCollection` (WGS84) into `(filter value, feature)`
/// rows, sorted by id. Property names are matched case-insensitively; ids
/// fall back to the feature `id`, then the name, then the feature's index.
pub fn read_rows(path: &Path, query: &LayerQuery) -> Result<Vec<(Option<String>, MapFeature)>> {
    rows_from_str(&std::fs::read_to_string(path)?, query)
}

/// [`read_rows`] for GeoJSON text already in memory. Accepts a
/// `FeatureCollection`, a single `Feature`, or a bare geometry.
pub fn rows_from_str(text: &str, query: &LayerQuery) -> Result<Vec<(Option<String>, MapFeature)>> {
    let mut rows: Vec<(Option<String>, MapFeature)> = raw_features(text, query)?
        .into_iter()
        .map(|r| {
            let feature = MapFeature {
                name: r.name.unwrap_or_else(|| r.id.clone()),
                names: r.names,
                id: r.id,
                class: query.class.clone(),
                // A parent's name means nothing without its code (Natural
                // Earth's `region` is a census region for US states).
                parent_name: r.parent.as_ref().and(r.parent_name),
                parent: r.parent,
                country: r.country.as_deref().and_then(crate::iso::country_alpha2),
                units: Vec::new(),
                wikidata: r.wikidata,
                geometry: into_multipolygon(r.geometry),
            };
            (r.filter, feature)
        })
        .collect();
    rows.sort_by(|a, b| a.1.id.cmp(&b.1.id));
    Ok(rows)
}

/// Reads the line features of a GeoJSON file (e.g. disputed boundaries),
/// sorted by id. Non-linear geometries are skipped.
pub fn read_lines(path: &Path, query: &LayerQuery) -> Result<Vec<MapLine>> {
    lines_from_str(&std::fs::read_to_string(path)?, query)
}

/// [`read_lines`] for GeoJSON text already in memory.
pub fn lines_from_str(text: &str, query: &LayerQuery) -> Result<Vec<MapLine>> {
    let mut lines: Vec<MapLine> = raw_features(text, query)?
        .into_iter()
        .map(|r| MapLine {
            name: r.name.unwrap_or_else(|| r.id.clone()),
            id: r.id,
            geometry: into_multilinestring(r.geometry),
        })
        .filter(|l| !l.geometry.0.is_empty())
        .collect();
    lines.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(lines)
}

/// A GeoJSON feature with all its properties (used by `mapgen convert`).
#[derive(Debug, Clone, PartialEq)]
pub struct Record {
    /// The feature's top-level `id`, if any.
    pub feature_id: Option<String>,
    pub properties: serde_json::Map<String, Value>,
    pub geometry: Geometry<f64>,
}

impl Record {
    /// A property as text (strings and numbers), matched case-insensitively.
    /// Blank and "no data" values (`NA`, `-99`) count as missing.
    pub fn prop(&self, name: &str) -> Option<String> {
        let v = self.properties.get(name).or_else(|| {
            self.properties
                .iter()
                .find(|(k, _)| k.eq_ignore_ascii_case(name))
                .map(|(_, v)| v)
        })?;
        meaningful(match v {
            Value::String(s) => Some(s.clone()),
            Value::Number(n) => Some(n.to_string()),
            _ => None,
        })
    }
}

/// Reads every feature of a GeoJSON file with all its properties.
pub fn read_records(path: &Path) -> Result<Vec<Record>> {
    records_from_str(&std::fs::read_to_string(path)?)
}

/// [`read_records`] for GeoJSON text already in memory. Accepts a
/// `FeatureCollection`, a single `Feature`, or a bare geometry.
pub fn records_from_str(text: &str) -> Result<Vec<Record>> {
    let collection = match text.parse::<::geojson::GeoJson>()? {
        ::geojson::GeoJson::FeatureCollection(fc) => fc,
        ::geojson::GeoJson::Feature(f) => ::geojson::FeatureCollection {
            bbox: None,
            features: vec![f],
            foreign_members: None,
        },
        ::geojson::GeoJson::Geometry(g) => ::geojson::FeatureCollection {
            bbox: None,
            features: vec![::geojson::Feature::from(g)],
            foreign_members: None,
        },
    };
    let mut out = Vec::new();
    for f in collection.features {
        let Some(geometry) = f.geometry else { continue };
        out.push(Record {
            feature_id: f.id.as_ref().map(feature_id_to_string),
            properties: f.properties.unwrap_or_default(),
            geometry: geometry.value.try_into()?,
        });
    }
    Ok(out)
}

fn raw_features(text: &str, query: &LayerQuery) -> Result<Vec<RawFeature>> {
    let languages = query.language_columns();
    Ok(records_from_str(text)?
        .into_iter()
        .enumerate()
        .map(|(i, r)| {
            let note = query.note_column.as_deref().and_then(|c| r.prop(c));
            let name = match (r.prop(&query.name_column), note) {
                (Some(n), Some(note)) => Some(format!("{n} — {note}")),
                (name, _) => name,
            };
            let id = query
                .id_columns
                .iter()
                .find_map(|c| r.prop(c))
                .or_else(|| r.feature_id.clone())
                .or_else(|| name.clone())
                .unwrap_or_else(|| i.to_string());
            RawFeature {
                filter: query.filter_column.as_deref().and_then(|c| r.prop(c)),
                parent: query.parent_column.as_deref().and_then(|c| r.prop(c)),
                parent_name: query.parent_name_column.as_deref().and_then(|c| r.prop(c)),
                country: query.country_column.as_deref().and_then(|c| r.prop(c)),
                wikidata: query
                    .wikidata_column
                    .as_deref()
                    .and_then(|c| r.prop(c))
                    .and_then(|v| crate::layer::wikidata_id(&v)),
                names: languages
                    .iter()
                    .filter_map(|(lang, c)| r.prop(c).map(|n| (lang.clone(), n)))
                    .collect(),
                name,
                id,
                geometry: r.geometry,
            }
        })
        .collect())
}

/// Convenience wrapper: reads every feature with explicit property names.
pub fn read_features(
    path: &Path,
    id_property: &str,
    name_property: &str,
    class: &str,
) -> Result<Vec<MapFeature>> {
    let query = LayerQuery {
        table: None,
        id_columns: vec![id_property.to_owned()],
        name_column: name_property.to_owned(),
        class: class.to_owned(),
        ..LayerQuery::default()
    };
    Ok(read_rows(path, &query)?
        .into_iter()
        .map(|(_, f)| f)
        .collect())
}

fn feature_id_to_string(id: &::geojson::feature::Id) -> String {
    match id {
        ::geojson::feature::Id::String(s) => s.clone(),
        ::geojson::feature::Id::Number(n) => n.to_string(),
    }
}
