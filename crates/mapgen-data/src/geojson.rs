use std::path::Path;

use geo_types::Geometry;
use mapgen_core::MapFeature;
use serde_json::Value;

use crate::error::Result;
use crate::geometry::into_multipolygon;
use crate::layer::{meaningful, LayerQuery};

/// Reads a GeoJSON `FeatureCollection` (WGS84) into `(filter value, feature)`
/// rows, sorted by id. Property names are matched case-insensitively; ids
/// fall back to the feature `id`, then the name, then the feature's index.
pub fn read_rows(path: &Path, query: &LayerQuery) -> Result<Vec<(Option<String>, MapFeature)>> {
    let text = std::fs::read_to_string(path)?;
    let collection: ::geojson::FeatureCollection = text.parse()?;

    let mut rows = Vec::new();
    for (i, f) in collection.features.into_iter().enumerate() {
        let Some(geometry) = f.geometry.clone() else {
            continue;
        };
        let prop = |name: &str| -> Option<String> {
            let props = f.properties.as_ref()?;
            let v = props.get(name).or_else(|| {
                props
                    .iter()
                    .find(|(k, _)| k.eq_ignore_ascii_case(name))
                    .map(|(_, v)| v)
            })?;
            meaningful(match v {
                Value::String(s) => Some(s.clone()),
                Value::Number(n) => Some(n.to_string()),
                _ => None,
            })
        };
        let name = prop(&query.name_column);
        let id = query
            .id_columns
            .iter()
            .find_map(|c| prop(c))
            .or_else(|| f.id.as_ref().map(feature_id_to_string))
            .or_else(|| name.clone())
            .unwrap_or_else(|| i.to_string());
        let filter = query.filter_column.as_deref().and_then(prop);
        let geometry: Geometry<f64> = geometry.value.try_into()?;
        rows.push((
            filter,
            MapFeature {
                name: name.unwrap_or_else(|| id.clone()),
                id,
                class: query.class.clone(),
                geometry: into_multipolygon(geometry),
            },
        ));
    }
    rows.sort_by(|a, b| a.1.id.cmp(&b.1.id));
    Ok(rows)
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
        filter_column: None,
        class: class.to_owned(),
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
