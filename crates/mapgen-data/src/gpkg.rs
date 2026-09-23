use std::path::Path;

use geozero::{wkb::GpkgWkb, ToGeo};
use mapgen_core::MapFeature;
use rusqlite::types::Value;
use rusqlite::{Connection, OpenFlags};

use crate::error::{Error, Result};
use crate::geometry::into_multipolygon;
use crate::layer::{meaningful, LayerQuery};

/// Reads features from a GeoPackage, optionally filtered to one region code.
///
/// The file is opened read-only and rows are streamed, so memory scales with
/// the selected region rather than the dataset.
pub fn read_features(
    path: &Path,
    query: &LayerQuery,
    region: Option<&str>,
) -> Result<Vec<MapFeature>> {
    let conn = open(path)?;
    let table = query.table.as_deref().ok_or(Error::MissingTable)?;
    let geom_col = geometry_column(&conn, table)?;
    let columns = table_columns(&conn, table)?;
    let has = |c: &str| columns.iter().any(|x| x.eq_ignore_ascii_case(c));

    let id_cols: Vec<&String> = query.id_columns.iter().filter(|c| has(c)).collect();
    if id_cols.is_empty() {
        return Err(Error::InvalidIdentifier(query.id_columns.join("|")));
    }
    let mut select = Vec::new();
    for c in &id_cols {
        select.push(ident(c)?);
    }
    select.push(ident(&query.name_column)?);
    select.push(ident(&geom_col)?);

    let mut sql = format!("SELECT {} FROM {}", select.join(", "), ident(table)?);
    let filter = match (&query.filter_column, region) {
        (Some(col), Some(code)) => {
            sql.push_str(&format!(" WHERE {} = ?1", ident(col)?));
            Some(code)
        }
        _ => None,
    };

    let mut stmt = conn.prepare(&sql)?;
    let mut rows = match filter {
        Some(code) => stmt.query([code])?,
        None => stmt.query([])?,
    };

    let n = id_cols.len();
    let mut features = Vec::new();
    while let Some(row) = rows.next()? {
        let Some(blob) = row.get::<_, Option<Vec<u8>>>(n + 1)? else {
            continue;
        };
        let ids = (0..n).map(|i| row.get::<_, Value>(i).map(value_to_string));
        let mut id = None;
        for v in ids {
            if let Some(v) = meaningful(v?) {
                id = Some(v);
                break;
            }
        }
        let name = meaningful(value_to_string(row.get(n)?));
        let Some(id) = id.or_else(|| name.clone()) else {
            continue;
        };
        let geometry = into_multipolygon(GpkgWkb(blob).to_geo()?);
        features.push(MapFeature {
            name: name.unwrap_or_else(|| id.clone()),
            id,
            class: query.class.clone(),
            geometry,
        });
    }
    features.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(features)
}

/// Distinct non-empty values of the query's filter column, sorted.
pub fn distinct_values(path: &Path, query: &LayerQuery) -> Result<Vec<String>> {
    let conn = open(path)?;
    let table = query.table.as_deref().ok_or(Error::MissingTable)?;
    let col = query
        .filter_column
        .as_deref()
        .ok_or(Error::NoFilterColumn(table.into()))?;
    let sql = format!(
        "SELECT DISTINCT {c} FROM {t} ORDER BY {c}",
        c = ident(col)?,
        t = ident(table)?
    );
    let mut stmt = conn.prepare(&sql)?;
    let values = stmt
        .query_map([], |r| r.get::<_, Value>(0))?
        .filter_map(|v| v.map(|v| meaningful(value_to_string(v))).transpose())
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(values)
}

fn open(path: &Path) -> Result<Connection> {
    Ok(Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?)
}

fn value_to_string(v: Value) -> Option<String> {
    match v {
        Value::Null | Value::Blob(_) => None,
        Value::Integer(i) => Some(i.to_string()),
        Value::Real(r) => Some(r.to_string()),
        Value::Text(s) => Some(s),
    }
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

fn table_columns(conn: &Connection, table: &str) -> Result<Vec<String>> {
    let mut stmt = conn.prepare(&format!("PRAGMA table_info({})", ident(table)?))?;
    let cols = stmt
        .query_map([], |r| r.get::<_, String>(1))?
        .collect::<rusqlite::Result<_>>()?;
    Ok(cols)
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
    use crate::Source;

    #[test]
    fn rejects_unsafe_identifiers() {
        assert!(ident("GID_1").is_ok());
        assert!(ident("x\"; DROP TABLE t; --").is_err());
        assert!(ident("").is_err());
    }

    #[test]
    fn natural_earth_layout() {
        let q = Source::NaturalEarthAdmin1.layer_query();
        assert_eq!(q.table.as_deref(), Some("ne_10m_admin_1_states_provinces"));
        assert_eq!(q.filter_column.as_deref(), Some("adm0_a3"));
    }
}
