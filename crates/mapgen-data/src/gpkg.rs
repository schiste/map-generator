use std::path::Path;

use geozero::{wkb::GpkgWkb, ToGeo};
use mapgen_core::MapFeature;
use rusqlite::{Connection, OpenFlags};

use crate::error::{Error, Result};
use crate::geometry::into_multipolygon;

/// Well-known layouts of the supported datasets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    /// GADM 4.1 "levels" GeoPackage (`gadm_410-levels.gpkg`), layers `ADM_0`..`ADM_5`.
    Gadm { level: u8 },
    /// Natural Earth vector GeoPackage, 1:10m Admin-0 countries.
    NaturalEarthAdmin0,
    /// Natural Earth vector GeoPackage, 1:10m Admin-1 states and provinces.
    NaturalEarthAdmin1,
}

/// A read query against one layer of a GeoPackage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LayerQuery {
    pub table: String,
    pub id_column: String,
    pub name_column: String,
    /// Column compared against the region code (e.g. ISO3 `FRA`).
    pub filter_column: Option<String>,
    pub class: String,
}

impl Source {
    pub fn layer_query(&self) -> LayerQuery {
        match *self {
            Source::Gadm { level: 0 } => LayerQuery {
                table: "ADM_0".into(),
                id_column: "GID_0".into(),
                name_column: "COUNTRY".into(),
                filter_column: Some("GID_0".into()),
                class: "country".into(),
            },
            Source::Gadm { level } => LayerQuery {
                table: format!("ADM_{level}"),
                id_column: format!("GID_{level}"),
                name_column: format!("NAME_{level}"),
                filter_column: Some("GID_0".into()),
                class: "subdivision".into(),
            },
            Source::NaturalEarthAdmin0 => LayerQuery {
                table: "ne_10m_admin_0_countries".into(),
                id_column: "ADM0_A3".into(),
                name_column: "NAME".into(),
                filter_column: Some("ADM0_A3".into()),
                class: "country".into(),
            },
            Source::NaturalEarthAdmin1 => LayerQuery {
                table: "ne_10m_admin_1_states_provinces".into(),
                id_column: "iso_3166_2".into(),
                name_column: "name".into(),
                filter_column: Some("adm0_a3".into()),
                class: "subdivision".into(),
            },
        }
    }
}

/// Reads features from a GeoPackage, optionally filtered to one region code.
///
/// The file is opened read-only; rows are streamed, so memory scales with the
/// selected region rather than the dataset.
pub fn read_features(
    path: &Path,
    query: &LayerQuery,
    region: Option<&str>,
) -> Result<Vec<MapFeature>> {
    let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let geom_col = geometry_column(&conn, &query.table)?;

    let mut sql = format!(
        "SELECT {}, {}, {} FROM {}",
        ident(&query.id_column)?,
        ident(&query.name_column)?,
        ident(&geom_col)?,
        ident(&query.table)?,
    );
    let filter = match (&query.filter_column, region) {
        (Some(col), Some(code)) => {
            sql.push_str(&format!(" WHERE {} = ?1", ident(col)?));
            Some(code)
        }
        _ => None,
    };
    sql.push_str(&format!(" ORDER BY {}", ident(&query.id_column)?));

    let mut stmt = conn.prepare(&sql)?;
    let mut rows = match filter {
        Some(code) => stmt.query([code])?,
        None => stmt.query([])?,
    };

    let mut features = Vec::new();
    while let Some(row) = rows.next()? {
        let id: String = row.get(0)?;
        let name: Option<String> = row.get(1)?;
        let blob: Option<Vec<u8>> = row.get(2)?;
        let Some(blob) = blob else { continue };
        let geometry = into_multipolygon(GpkgWkb(blob).to_geo()?);
        features.push(MapFeature {
            name: name.unwrap_or_else(|| id.clone()),
            id,
            class: query.class.clone(),
            geometry,
        });
    }
    Ok(features)
}

fn geometry_column(conn: &Connection, table: &str) -> Result<String> {
    conn.query_row(
        "SELECT column_name FROM gpkg_geometry_columns WHERE table_name = ?1 COLLATE NOCASE",
        [table],
        |r| r.get(0),
    )
    .map_err(|e| match e {
        rusqlite::Error::QueryReturnedNoRows => Error::UnknownLayer(table.into()),
        e => e.into(),
    })
}

/// Table and column names cannot be bound as SQL parameters, so they are
/// restricted to `[A-Za-z0-9_]` and double-quoted.
fn ident(name: &str) -> Result<String> {
    if !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        Ok(format!("\"{name}\""))
    } else {
        Err(Error::InvalidIdentifier(name.into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_unsafe_identifiers() {
        assert!(ident("GID_1").is_ok());
        assert!(ident("x\"; DROP TABLE t; --").is_err());
        assert!(ident("").is_err());
    }

    #[test]
    fn gadm_level_layout() {
        let q = Source::Gadm { level: 2 }.layer_query();
        assert_eq!((q.table.as_str(), q.id_column.as_str()), ("ADM_2", "GID_2"));
    }
}
