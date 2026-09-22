use std::path::Path;

use geo_types::Geometry;
use mapgen_core::MapFeature;

use crate::error::Result;
use crate::geometry::into_multipolygon;

/// Reads a GeoJSON `FeatureCollection` (WGS84). Mainly for fixtures and
/// small hand-made inputs.
///
/// The id comes from the `id_property` property, falling back to the feature
/// `id`, then to the feature's index. The name comes from `name_property`.
pub fn read_features(
    path: &Path,
    id_property: &str,
    name_property: &str,
    class: &str,
) -> Result<Vec<MapFeature>> {
    let text = std::fs::read_to_string(path)?;
    let collection: ::geojson::FeatureCollection = text.parse()?;

    let mut features = Vec::new();
    for (i, f) in collection.features.into_iter().enumerate() {
        let Some(geometry) = f.geometry.clone() else {
            continue;
        };
        let id = f
            .property(id_property)
            .and_then(|v| v.as_str().map(str::to_owned))
            .or_else(|| f.id.as_ref().map(feature_id_to_string))
            .unwrap_or_else(|| i.to_string());
        let name = f
            .property(name_property)
            .and_then(|v| v.as_str().map(str::to_owned))
            .unwrap_or_else(|| id.clone());
        let geometry: Geometry<f64> = geometry.value.try_into()?;
        features.push(MapFeature {
            id,
            name,
            class: class.to_owned(),
            geometry: into_multipolygon(geometry),
        });
    }
    features.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(features)
}

fn feature_id_to_string(id: &::geojson::feature::Id) -> String {
    match id {
        ::geojson::feature::Id::String(s) => s.clone(),
        ::geojson::feature::Id::Number(n) => n.to_string(),
    }
}
