//! Writes an indexed GeoPackage (`mapgen convert`).
//!
//! GeoJSON has no index, so every read parses the whole file. A GeoPackage
//! written here has an R-tree spatial index (the standard GeoPackage
//! extension) and B-tree indexes on the region and id columns, so reading
//! one country or the neighbours of a region is near-instant.

use std::collections::BTreeMap;
use std::path::Path;

use geo::BoundingRect;
use geo_types::{Geometry, LineString, MultiLineString, MultiPolygon, Polygon};
use rusqlite::types::Value as Sql;
use rusqlite::{params_from_iter, Connection};
use serde_json::Value;

use crate::error::Result;
use crate::geojson::Record;

pub struct WriteOptions {
    /// Feature table name (becomes the GeoPackage layer name).
    pub table: String,
    /// Columns to index (e.g. the region and id columns); missing ones are skipped.
    pub index_columns: Vec<String>,
}

/// Writes `records` to a new GeoPackage at `path` (replacing any file there).
/// Every property becomes a column; names are sanitised to `[A-Za-z0-9_]`.
pub fn write_gpkg(path: &Path, records: &[Record], opts: &WriteOptions) -> Result<()> {
    let table = sanitize(&opts.table);
    if path.exists() {
        std::fs::remove_file(path)?;
    }
    let mut conn = Connection::open(path)?;

    // Property columns in first-seen order, deduplicated case-insensitively.
    let mut columns: Vec<(String, String)> = Vec::new(); // (property key, column)
    let mut taken: BTreeMap<String, ()> = BTreeMap::new();
    for r in records {
        for key in r.properties.keys() {
            let col = sanitize(key);
            let lower = col.to_ascii_lowercase();
            if lower == "fid" || lower == "geom" || taken.contains_key(&lower) {
                continue;
            }
            taken.insert(lower, ());
            columns.push((key.clone(), col));
        }
    }

    let tx = conn.transaction()?;
    tx.execute_batch(&format!(
        "PRAGMA application_id = 1196444487;
         PRAGMA user_version = 10300;
         CREATE TABLE gpkg_spatial_ref_sys (srs_name TEXT NOT NULL, srs_id INTEGER PRIMARY KEY, organization TEXT NOT NULL,
           organization_coordsys_id INTEGER NOT NULL, definition TEXT NOT NULL, description TEXT);
         INSERT INTO gpkg_spatial_ref_sys VALUES
           ('Undefined cartesian SRS', -1, 'NONE', -1, 'undefined', NULL),
           ('Undefined geographic SRS', 0, 'NONE', 0, 'undefined', NULL),
           ('WGS 84 geodetic', 4326, 'EPSG', 4326, '{WGS84}', 'longitude/latitude coordinates in decimal degrees on the WGS 84 spheroid');
         CREATE TABLE gpkg_contents (table_name TEXT NOT NULL PRIMARY KEY, data_type TEXT NOT NULL, identifier TEXT UNIQUE,
           description TEXT DEFAULT '', last_change DATETIME NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
           min_x DOUBLE, min_y DOUBLE, max_x DOUBLE, max_y DOUBLE, srs_id INTEGER);
         CREATE TABLE gpkg_geometry_columns (table_name TEXT NOT NULL, column_name TEXT NOT NULL, geometry_type_name TEXT NOT NULL,
           srs_id INTEGER NOT NULL, z TINYINT NOT NULL, m TINYINT NOT NULL, PRIMARY KEY (table_name, column_name));
         CREATE TABLE gpkg_extensions (table_name TEXT, column_name TEXT, extension_name TEXT NOT NULL, definition TEXT NOT NULL, scope TEXT NOT NULL);",
        WGS84 = WGS84_WKT.replace('\'', "''"),
    ))?;
    let col_defs: String = columns.iter().map(|(_, c)| format!(", \"{c}\"")).collect();
    tx.execute_batch(&format!(
        "CREATE TABLE \"{table}\" (fid INTEGER PRIMARY KEY AUTOINCREMENT, geom BLOB{col_defs});
         CREATE VIRTUAL TABLE \"rtree_{table}_geom\" USING rtree(id, minx, maxx, miny, maxy);"
    ))?;

    let (mut min_x, mut min_y, mut max_x, mut max_y) = (
        f64::INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NEG_INFINITY,
    );
    {
        let placeholders: String = std::iter::repeat_n(", ?", columns.len()).collect();
        let mut insert = tx.prepare(&format!(
            "INSERT INTO \"{table}\" (geom{}) VALUES (?{placeholders})",
            columns
                .iter()
                .map(|(_, c)| format!(", \"{c}\""))
                .collect::<String>()
        ))?;
        let mut rtree = tx.prepare(&format!(
            "INSERT INTO \"rtree_{table}_geom\" VALUES (?, ?, ?, ?, ?)"
        ))?;
        for r in records {
            let bbox = r.geometry.bounding_rect();
            let mut values: Vec<Sql> = vec![Sql::Blob(gpkg_blob(
                &r.geometry,
                bbox.map(|b| [b.min().x, b.max().x, b.min().y, b.max().y]),
            ))];
            for (key, _) in &columns {
                values.push(json_to_sql(r.properties.get(key)));
            }
            insert.execute(params_from_iter(values))?;
            if let Some(b) = bbox {
                let fid = tx.last_insert_rowid();
                rtree.execute(rusqlite::params![
                    fid,
                    b.min().x,
                    b.max().x,
                    b.min().y,
                    b.max().y
                ])?;
                min_x = min_x.min(b.min().x);
                min_y = min_y.min(b.min().y);
                max_x = max_x.max(b.max().x);
                max_y = max_y.max(b.max().y);
            }
        }
    }
    let finite = |v: f64| v.is_finite().then_some(v);
    tx.execute(
        "INSERT INTO gpkg_contents (table_name, data_type, identifier, min_x, min_y, max_x, max_y, srs_id) VALUES (?1, 'features', ?1, ?2, ?3, ?4, ?5, 4326)",
        rusqlite::params![table, finite(min_x), finite(min_y), finite(max_x), finite(max_y)],
    )?;
    tx.execute(
        "INSERT INTO gpkg_geometry_columns VALUES (?1, 'geom', 'GEOMETRY', 4326, 0, 0)",
        [&table],
    )?;
    tx.execute(
        "INSERT INTO gpkg_extensions VALUES (?1, 'geom', 'gpkg_rtree_index', 'http://www.geopackage.org/spec120/#extension_rtree', 'write-only')",
        [&table],
    )?;
    // The same column may be asked for twice (e.g. an id that is also the
    // region column); index it once.
    let mut indexed: Vec<&str> = Vec::new();
    for wanted in &opts.index_columns {
        let wanted = sanitize(wanted);
        let Some((_, col)) = columns
            .iter()
            .find(|(_, c)| c.eq_ignore_ascii_case(&wanted))
        else {
            continue;
        };
        if indexed.contains(&col.as_str()) {
            continue;
        }
        indexed.push(col);
        tx.execute_batch(&format!(
            "CREATE INDEX \"idx_{table}_{col}\" ON \"{table}\"(\"{col}\");"
        ))?;
    }
    tx.commit()?;
    Ok(())
}

fn sanitize(name: &str) -> String {
    let s: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if s.is_empty() || s.starts_with(|c: char| c.is_ascii_digit()) {
        format!("c_{s}")
    } else {
        s
    }
}

fn json_to_sql(v: Option<&Value>) -> Sql {
    match v {
        None | Some(Value::Null) => Sql::Null,
        Some(Value::Bool(b)) => Sql::Integer(i64::from(*b)),
        Some(Value::Number(n)) => n
            .as_i64()
            .map_or_else(|| Sql::Real(n.as_f64().unwrap_or(f64::NAN)), Sql::Integer),
        Some(Value::String(s)) => Sql::Text(s.clone()),
        Some(other) => Sql::Text(other.to_string()),
    }
}

/// GeoPackage geometry blob: `GP` header (version 0, little-endian, XY
/// envelope, SRS 4326) followed by little-endian WKB.
fn gpkg_blob(g: &Geometry<f64>, envelope: Option<[f64; 4]>) -> Vec<u8> {
    let mut b = vec![b'G', b'P', 0];
    match envelope {
        Some(env) => {
            b.push(0b0000_0011);
            b.extend(4326i32.to_le_bytes());
            for v in env {
                b.extend(v.to_le_bytes());
            }
        }
        None => {
            b.push(0b0001_0001); // empty geometry flag, no envelope
            b.extend(4326i32.to_le_bytes());
        }
    }
    wkb(&mut b, g);
    b
}

fn wkb(b: &mut Vec<u8>, g: &Geometry<f64>) {
    match g {
        Geometry::Polygon(p) => wkb_multipolygon(b, &MultiPolygon(vec![p.clone()])),
        Geometry::MultiPolygon(mp) => wkb_multipolygon(b, mp),
        Geometry::LineString(l) => wkb_multilinestring(b, &MultiLineString(vec![l.clone()])),
        Geometry::MultiLineString(ml) => wkb_multilinestring(b, ml),
        Geometry::GeometryCollection(gc) => {
            header(b, 7, gc.0.len());
            for g in &gc.0 {
                wkb(b, g);
            }
        }
        // Points and other types are not used by map-generator: an empty collection.
        _ => header(b, 7, 0),
    }
}

fn header(b: &mut Vec<u8>, kind: u32, n: usize) {
    b.push(1);
    b.extend(kind.to_le_bytes());
    b.extend((n as u32).to_le_bytes());
}

fn wkb_ring(b: &mut Vec<u8>, r: &LineString<f64>) {
    b.extend((r.0.len() as u32).to_le_bytes());
    for c in &r.0 {
        b.extend(c.x.to_le_bytes());
        b.extend(c.y.to_le_bytes());
    }
}

fn wkb_polygon(b: &mut Vec<u8>, p: &Polygon<f64>) {
    header(b, 3, 1 + p.interiors().len());
    wkb_ring(b, p.exterior());
    for h in p.interiors() {
        wkb_ring(b, h);
    }
}

fn wkb_multipolygon(b: &mut Vec<u8>, mp: &MultiPolygon<f64>) {
    header(b, 6, mp.0.len());
    for p in &mp.0 {
        wkb_polygon(b, p);
    }
}

fn wkb_multilinestring(b: &mut Vec<u8>, ml: &MultiLineString<f64>) {
    header(b, 5, ml.0.len());
    for l in &ml.0 {
        b.push(1);
        b.extend(2u32.to_le_bytes());
        wkb_ring(b, l);
    }
}

const WGS84_WKT: &str = r#"GEOGCS["WGS 84",DATUM["WGS_1984",SPHEROID["WGS 84",6378137,298.257223563,AUTHORITY["EPSG","7030"]],AUTHORITY["EPSG","6326"]],PRIMEM["Greenwich",0,AUTHORITY["EPSG","8901"]],UNIT["degree",0.0174532925199433,AUTHORITY["EPSG","9122"]],AUTHORITY["EPSG","4326"]]"#;
