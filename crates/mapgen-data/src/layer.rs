use std::collections::BTreeMap;
use std::path::Path;

use mapgen_core::MapFeature;

use crate::error::{Error, Result};
use crate::{geojson, gpkg};

/// What to read from a layer. Column names double as GeoJSON property names
/// (matched case-insensitively).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LayerQuery {
    /// GeoPackage table; ignored for GeoJSON.
    pub table: Option<String>,
    /// Columns tried in order for the id; the first non-empty value wins
    /// (`NA` counts as empty). Missing columns are skipped.
    pub id_columns: Vec<String>,
    pub name_column: String,
    /// Column compared against the region code (e.g. ISO3 `FRA`).
    pub filter_column: Option<String>,
    /// CSS class emitted on each path.
    pub class: String,
}

/// Well-known layouts of the supported datasets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    /// GADM 4.1 "levels" GeoPackage (`gadm_410-levels.gpkg`), layers `ADM_0`..`ADM_5`.
    Gadm { level: u8 },
    /// Natural Earth 1:10m Admin-0 countries.
    NaturalEarthAdmin0,
    /// Natural Earth 1:10m Admin-1 states, provinces, départements...
    NaturalEarthAdmin1,
    /// Natural Earth 1:10m lakes.
    NaturalEarthLakes,
}

impl Source {
    pub fn layer_query(&self) -> LayerQuery {
        let q = |table: String, ids: &[&str], name: &str, filter: Option<&str>, class: &str| {
            LayerQuery {
                table: Some(table),
                id_columns: ids.iter().map(|s| s.to_string()).collect(),
                name_column: name.into(),
                filter_column: filter.map(Into::into),
                class: class.into(),
            }
        };
        match *self {
            Source::Gadm { level: 0 } => q(
                "ADM_0".into(),
                &["GID_0"],
                "COUNTRY",
                Some("GID_0"),
                "country",
            ),
            // GADM 4.1 carries ISO 3166-2 codes at level 1 only.
            Source::Gadm { level: 1 } => q(
                "ADM_1".into(),
                &["ISO_1", "GID_1"],
                "NAME_1",
                Some("GID_0"),
                "subdivision",
            ),
            Source::Gadm { level } => q(
                format!("ADM_{level}"),
                &[&format!("GID_{level}")],
                &format!("NAME_{level}"),
                Some("GID_0"),
                "subdivision",
            ),
            Source::NaturalEarthAdmin0 => q(
                "ne_10m_admin_0_countries".into(),
                &["ADM0_A3"],
                "NAME",
                Some("ADM0_A3"),
                "country",
            ),
            Source::NaturalEarthAdmin1 => q(
                "ne_10m_admin_1_states_provinces".into(),
                &["iso_3166_2", "adm1_code"],
                "name",
                Some("adm0_a3"),
                "subdivision",
            ),
            Source::NaturalEarthLakes => q(
                "ne_10m_lakes".into(),
                &["ne_id", "name"],
                "name",
                None,
                "lake",
            ),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    GeoPackage,
    GeoJson,
}

impl Format {
    pub fn of(path: &Path) -> Result<Format> {
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        match ext.as_str() {
            "gpkg" | "sqlite" => Ok(Format::GeoPackage),
            "geojson" | "json" => Ok(Format::GeoJson),
            _ => Err(Error::UnsupportedFormat(path.display().to_string())),
        }
    }
}

/// Reads a layer, optionally keeping only rows whose filter column equals `region`.
pub fn read_layer(
    path: &Path,
    query: &LayerQuery,
    region: Option<&str>,
) -> Result<Vec<MapFeature>> {
    match Format::of(path)? {
        Format::GeoPackage => gpkg::read_features(path, query, region),
        Format::GeoJson => {
            let region = region.filter(|_| query.filter_column.is_some());
            let mut rows = geojson::read_rows(path, query)?;
            if let Some(code) = region {
                rows.retain(|(value, _)| value.as_deref() == Some(code));
            }
            Ok(rows.into_iter().map(|(_, f)| f).collect())
        }
    }
}

/// Distinct values of the filter column (e.g. every country code), sorted.
pub fn list_regions(path: &Path, query: &LayerQuery) -> Result<Vec<String>> {
    if query.filter_column.is_none() {
        return Err(Error::NoFilterColumn(path.display().to_string()));
    }
    match Format::of(path)? {
        Format::GeoPackage => gpkg::distinct_values(path, query),
        Format::GeoJson => Ok(read_grouped(path, query)?.into_keys().collect()),
    }
}

/// Reads a whole GeoJSON layer grouped by filter value (for batch jobs).
pub fn read_grouped(path: &Path, query: &LayerQuery) -> Result<BTreeMap<String, Vec<MapFeature>>> {
    let mut groups: BTreeMap<String, Vec<MapFeature>> = BTreeMap::new();
    for (value, f) in geojson::read_rows(path, query)? {
        if let Some(v) = value {
            groups.entry(v).or_default().push(f);
        }
    }
    Ok(groups)
}

/// `NA` and blank values count as missing, as in GADM.
pub(crate) fn meaningful(v: Option<String>) -> Option<String> {
    v.map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty() && s != "NA" && s != "-99")
}
