use std::collections::BTreeMap;
use std::path::Path;

use mapgen_core::MapFeature;

use crate::error::{Error, Result};
use crate::geojson;

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
    /// Column holding the code of the enclosing unit (see `MapFeature::parent`).
    pub parent_column: Option<String>,
    /// CSS class emitted on each path.
    pub class: String,
}

/// Well-known layouts of the supported datasets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    /// geoBoundaries gbOpen GeoJSON (one file per country and level).
    GeoBoundaries,
    /// Natural Earth 1:10m Admin-0 countries.
    NaturalEarthAdmin0,
    /// Natural Earth 1:10m Admin-1 states, provinces, départements...
    NaturalEarthAdmin1,
    /// Natural Earth 1:10m lakes.
    NaturalEarthLakes,
    /// Natural Earth 1:10m disputed and claimed boundary lines.
    NaturalEarthDisputedLines,
}

impl Source {
    pub fn layer_query(&self) -> LayerQuery {
        let q = |table: String, ids: &[&str], name: &str, filter: Option<&str>, class: &str| {
            LayerQuery {
                table: Some(table),
                id_columns: ids.iter().map(|s| s.to_string()).collect(),
                name_column: name.into(),
                filter_column: filter.map(Into::into),
                parent_column: None,
                class: class.into(),
            }
        };
        match *self {
            // `code`/`parent` are added by `mapgen convert --ids-from`; `shapeISO`
            // is only filled for some layers; `shapeID` is always set.
            Source::GeoBoundaries => LayerQuery {
                table: None,
                parent_column: Some("parent".into()),
                ..q(
                    String::new(),
                    &["code", "shapeISO", "shapeID"],
                    "shapeName",
                    Some("shapeGroup"),
                    "subdivision",
                )
            },
            Source::NaturalEarthAdmin0 => q(
                "ne_10m_admin_0_countries".into(),
                &["ADM0_A3"],
                "NAME",
                Some("ADM0_A3"),
                "country",
            ),
            // `region_cod` groups e.g. French départements into régions.
            Source::NaturalEarthAdmin1 => LayerQuery {
                parent_column: Some("region_cod".into()),
                ..q(
                    "ne_10m_admin_1_states_provinces".into(),
                    &["iso_3166_2", "adm1_code"],
                    "name",
                    Some("adm0_a3"),
                    "subdivision",
                )
            },
            Source::NaturalEarthLakes => q(
                "ne_10m_lakes".into(),
                &["ne_id", "name"],
                "name",
                None,
                "lake",
            ),
            Source::NaturalEarthDisputedLines => q(
                "ne_10m_admin_0_boundary_lines_disputed_areas".into(),
                &["ne_id"],
                "NAME",
                None,
                "disputed",
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
        Format::GeoPackage => read_gpkg(path, query, region),
        Format::GeoJson => Ok(filter_rows(geojson::read_rows(path, query)?, query, region)),
    }
}

/// [`read_layer`] limited to features whose bounding box intersects `bbox`
/// (`[min_lon, min_lat, max_lon, max_lat]`). Fast on GeoPackages with an
/// R-tree index; GeoJSON is still parsed in full, then filtered.
pub fn read_layer_in(
    path: &Path,
    query: &LayerQuery,
    region: Option<&str>,
    bbox: Option<[f64; 4]>,
) -> Result<Vec<MapFeature>> {
    match Format::of(path)? {
        #[cfg(feature = "gpkg")]
        Format::GeoPackage => crate::gpkg::read_features_in(path, query, region, bbox),
        #[cfg(not(feature = "gpkg"))]
        Format::GeoPackage => Err(Error::GeoPackageUnsupported),
        Format::GeoJson => {
            let mut v = read_layer(path, query, region)?;
            if let Some(b) = bbox {
                v.retain(|f| {
                    geo::BoundingRect::bounding_rect(&f.geometry).is_some_and(|r| {
                        r.min().x <= b[2]
                            && r.max().x >= b[0]
                            && r.min().y <= b[3]
                            && r.max().y >= b[1]
                    })
                });
            }
            Ok(v)
        }
    }
}

/// [`read_layer`] for GeoJSON text already in memory (e.g. in a browser).
pub fn read_layer_str(
    text: &str,
    query: &LayerQuery,
    region: Option<&str>,
) -> Result<Vec<MapFeature>> {
    Ok(filter_rows(
        geojson::rows_from_str(text, query)?,
        query,
        region,
    ))
}

fn filter_rows(
    rows: Vec<(Option<String>, MapFeature)>,
    query: &LayerQuery,
    region: Option<&str>,
) -> Vec<MapFeature> {
    let region = region.filter(|_| query.filter_column.is_some());
    rows.into_iter()
        .filter(|(value, _)| region.is_none() || value.as_deref() == region)
        .map(|(_, f)| f)
        .collect()
}

#[cfg(feature = "gpkg")]
fn read_gpkg(path: &Path, query: &LayerQuery, region: Option<&str>) -> Result<Vec<MapFeature>> {
    crate::gpkg::read_features(path, query, region)
}

#[cfg(not(feature = "gpkg"))]
fn read_gpkg(_: &Path, _: &LayerQuery, _: Option<&str>) -> Result<Vec<MapFeature>> {
    Err(Error::GeoPackageUnsupported)
}

/// Distinct values of the filter column (e.g. every country code), sorted.
pub fn list_regions(path: &Path, query: &LayerQuery) -> Result<Vec<String>> {
    if query.filter_column.is_none() {
        return Err(Error::NoFilterColumn(path.display().to_string()));
    }
    match Format::of(path)? {
        #[cfg(feature = "gpkg")]
        Format::GeoPackage => crate::gpkg::distinct_values(path, query),
        #[cfg(not(feature = "gpkg"))]
        Format::GeoPackage => Err(Error::GeoPackageUnsupported),
        Format::GeoJson => Ok(read_grouped(path, query)?.into_keys().collect()),
    }
}

/// Reads a whole GeoJSON layer grouped by filter value (for batch jobs).
pub fn read_grouped(path: &Path, query: &LayerQuery) -> Result<BTreeMap<String, Vec<MapFeature>>> {
    Ok(group_rows(geojson::read_rows(path, query)?))
}

/// Groups `(filter value, feature)` rows by value; rows without one are dropped.
pub fn group_rows(rows: Vec<(Option<String>, MapFeature)>) -> BTreeMap<String, Vec<MapFeature>> {
    let mut groups: BTreeMap<String, Vec<MapFeature>> = BTreeMap::new();
    for (value, f) in rows {
        if let Some(v) = value {
            groups.entry(v).or_default().push(f);
        }
    }
    groups
}

/// Blank values and common "no data" markers (`NA`, `-99`) count as missing.
pub(crate) fn meaningful(v: Option<String>) -> Option<String> {
    v.map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty() && s != "NA" && s != "-99")
}
