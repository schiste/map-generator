//! Capitals from Natural Earth's populated places.
use std::collections::BTreeMap;
use std::path::Path;

use mapgen_core::{MapPlace, PlaceKind};
use serde_json::Value;

use crate::error::{Error, Result};
use crate::layer::{language_column, wikidata_id};

/// Reads the capitals of a Natural Earth populated places GeoJSON (the full
/// file or `ne_10m_capitals.geojson`, its capitals only), sorted by id, with
/// names in `languages` (`NAME_FR`...). Other places are skipped.
pub fn read_places(path: &Path, languages: &[String]) -> Result<Vec<MapPlace>> {
    places_from_str(&std::fs::read_to_string(path)?, languages)
}

/// [`read_places`] for GeoJSON text already in memory.
pub fn places_from_str(text: &str, languages: &[String]) -> Result<Vec<MapPlace>> {
    let doc: Value = serde_json::from_str(text)?;
    let features = doc
        .get("features")
        .and_then(Value::as_array)
        .ok_or_else(|| Error::Table("a places file is a GeoJSON FeatureCollection".into()))?;
    let mut places: Vec<MapPlace> = features
        .iter()
        .filter_map(|f| place(f, languages))
        .collect();
    places.sort_by(|a, b| a.id.cmp(&b.id));
    places.dedup_by(|a, b| a.id == b.id);
    Ok(places)
}

/// Natural Earth's `FEATURECLA` of a capital, as a [`PlaceKind`].
pub fn place_kind(featurecla: &str) -> Option<PlaceKind> {
    match featurecla.trim().to_ascii_lowercase().as_str() {
        "admin-0 capital" | "admin-0 capital alt" => Some(PlaceKind::CountryCapital),
        "admin-1 capital" | "admin-1 region capital" | "admin-0 region capital" => {
            Some(PlaceKind::RegionCapital)
        }
        _ => None,
    }
}

fn place(f: &Value, languages: &[String]) -> Option<MapPlace> {
    let props = f.get("properties")?.as_object()?;
    let get = |key: &str| {
        props
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(key))
            .and_then(|(_, v)| match v {
                Value::String(s) if !s.trim().is_empty() => Some(s.trim().to_owned()),
                Value::Number(n) => Some(n.to_string()),
                _ => None,
            })
    };
    let kind = place_kind(&get("FEATURECLA")?)?;
    let coords = f.get("geometry")?.get("coordinates")?.as_array()?;
    let (lon, lat) = (coords.first()?.as_f64()?, coords.get(1)?.as_f64()?);
    let name = get("NAME")?;
    let names: BTreeMap<String, String> = languages
        .iter()
        .filter_map(|l| Some((l.clone(), get(&language_column("NAME_{lang}", l))?)))
        .collect();
    Some(MapPlace {
        id: get("NE_ID").unwrap_or_else(|| name.clone()),
        names,
        kind,
        country: get("ADM0_A3")
            .as_deref()
            .and_then(crate::iso::country_alpha2),
        wikidata: get("WIKIDATAID").as_deref().and_then(wikidata_id),
        lon,
        lat,
        name,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_capitals_only() {
        let text = r#"{"type":"FeatureCollection","features":[
          {"type":"Feature","properties":{"FEATURECLA":"Admin-0 capital","NAME":"Paris","NAME_DE":"Paris","NAME_JA":"パリ","NAME_ZHT":"巴黎","ADM0_A3":"FRA","WIKIDATAID":"Q90","NE_ID":1159151613},"geometry":{"type":"Point","coordinates":[2.35,48.86]}},
          {"type":"Feature","properties":{"FEATURECLA":"Admin-1 capital","NAME":"Lyon","ADM0_A3":"FRA","NE_ID":1159150999},"geometry":{"type":"Point","coordinates":[4.83,45.76]}},
          {"type":"Feature","properties":{"FEATURECLA":"Populated place","NAME":"Nowhere","NE_ID":3},"geometry":{"type":"Point","coordinates":[0,0]}}
        ]}"#;
        let places = places_from_str(text, &["ja".into(), "zh-Hant".into()]).unwrap();
        assert_eq!(places.len(), 2);
        let lyon = &places[0];
        assert_eq!(
            (lyon.name.as_str(), lyon.kind),
            ("Lyon", PlaceKind::RegionCapital)
        );
        let paris = &places[1];
        assert_eq!(paris.kind, PlaceKind::CountryCapital);
        assert_eq!(paris.country.as_deref(), Some("fr"));
        assert_eq!(paris.wikidata.as_deref(), Some("Q90"));
        assert_eq!(paris.names.get("ja").map(String::as_str), Some("パリ"));
        assert_eq!(paris.names.get("zh-Hant").map(String::as_str), Some("巴黎"));
        assert!(!paris.names.contains_key("de"), "only the asked languages");
        assert_eq!((paris.lon, paris.lat), (2.35, 48.86));
    }
}
