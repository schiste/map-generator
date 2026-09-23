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
    read_features_in(path, query, region, None)
}

/// [`read_features`] limited to features whose bounding box intersects
/// `bbox` (`[min_lon, min_lat, max_lon, max_lat]`). Uses the GeoPackage R-tree
/// index when the file has one (as files written by `mapgen convert` do).
pub fn read_features_in(
    path: &Path,
    query: &LayerQuery,
    region: Option<&str>,
    bbox: Option<[f64; 4]>,
) -> Result<Vec<MapFeature>> {
    let conn = open(path)?;
    let table = resolve_table(&conn, query)?;
    let table = table.as_str();
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
    for optional in [
        &query.parent_column,
        &query.parent_name_column,
        &query.country_column,
    ] {
        match optional.as_deref().filter(|c| has(c)) {
            Some(c) => select.push(ident(c)?),
            None => select.push("NULL".into()),
        }
    }
    select.push(ident(&geom_col)?);

    let mut sql = format!("SELECT {} FROM {}", select.join(", "), ident(table)?);
    let mut conditions: Vec<String> = Vec::new();
    let mut params: Vec<Value> = Vec::new();
    if let (Some(col), Some(code)) = (&query.filter_column, region) {
        params.push(Value::Text(code.to_owned()));
        conditions.push(format!("{} = ?{}", ident(col)?, params.len()));
    }
    if let (Some(b), Some((rtree, pk))) = (bbox, rtree_index(&conn, table, &geom_col)?) {
        let n = params.len();
        params.extend([b[2], b[0], b[3], b[1]].map(Value::Real));
        conditions.push(format!(
            "{} IN (SELECT id FROM {} WHERE minx <= ?{} AND maxx >= ?{} AND miny <= ?{} AND maxy >= ?{})",
            ident(&pk)?,
            ident(&rtree)?,
            n + 1,
            n + 2,
            n + 3,
            n + 4
        ));
    }
    if !conditions.is_empty() {
        sql.push_str(" WHERE ");
        sql.push_str(&conditions.join(" AND "));
    }

    let mut stmt = conn.prepare(&sql)?;
    let mut rows = stmt.query(rusqlite::params_from_iter(params))?;

    let n = id_cols.len();
    let mut features = Vec::new();
    while let Some(row) = rows.next()? {
        let Some(blob) = row.get::<_, Option<Vec<u8>>>(n + 4)? else {
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
        let parent = meaningful(value_to_string(row.get(n + 1)?));
        let parent_name = parent
            .as_ref()
            .and(meaningful(value_to_string(row.get(n + 2)?)));
        let country = meaningful(value_to_string(row.get(n + 3)?))
            .and_then(|c| crate::iso::country_alpha2(&c));
        let Some(id) = id.or_else(|| name.clone()) else {
            continue;
        };
        let geometry = into_multipolygon(GpkgWkb(blob).to_geo()?);
        features.push(MapFeature {
            name: name.unwrap_or_else(|| id.clone()),
            id,
            class: query.class.clone(),
            parent,
            parent_name,
            country,
            geometry,
        });
    }
    features.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(features)
}

/// Distinct non-empty values of the query's filter column, sorted.
pub fn distinct_values(path: &Path, query: &LayerQuery) -> Result<Vec<String>> {
    let conn = open(path)?;
    let table = resolve_table(&conn, query)?;
    let table = table.as_str();
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

/// The query's table, or the only feature table of the file (e.g. one written
/// by `mapgen convert` from a geoBoundaries GeoJSON).
fn resolve_table(conn: &Connection, query: &LayerQuery) -> Result<String> {
    if let Some(t) = &query.table {
        return Ok(t.clone());
    }
    let mut stmt =
        conn.prepare("SELECT table_name FROM gpkg_geometry_columns ORDER BY table_name")?;
    let tables = stmt
        .query_map([], |r| r.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    match tables.as_slice() {
        [only] => Ok(only.clone()),
        _ => Err(Error::MissingTable),
    }
}

/// The R-tree index table and primary-key column of a feature table, if the
/// file has the standard GeoPackage R-tree extension for it.
fn rtree_index(conn: &Connection, table: &str, geom: &str) -> Result<Option<(String, String)>> {
    let name = format!("rtree_{table}_{geom}");
    let exists: bool = conn.query_row(
        "SELECT count(*) > 0 FROM sqlite_master WHERE name = ?1 COLLATE NOCASE",
        [&name],
        |r| r.get(0),
    )?;
    if !exists {
        return Ok(None);
    }
    let mut stmt = conn.prepare(&format!("PRAGMA table_info({})", ident(table)?))?;
    let pk = stmt
        .query_map([], |r| Ok((r.get::<_, String>(1)?, r.get::<_, i64>(5)?)))?
        .filter_map(|r| r.ok())
        .find(|(_, pk)| *pk == 1)
        .map(|(name, _)| name);
    Ok(pk.map(|pk| (name, pk)))
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
