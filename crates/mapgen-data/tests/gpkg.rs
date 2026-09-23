//! Reads a tiny GeoPackage built on the fly, so the SQL path is tested
//! without downloading any dataset.
#![cfg(feature = "gpkg")]

use std::path::PathBuf;

use mapgen_data::{list_regions, read_layer, LayerQuery};
use rusqlite::Connection;

/// GeoPackage geometry blob: `GP` header (v0, little-endian, no envelope,
/// SRS 4326) followed by a WKB polygon.
fn gpkg_polygon(coords: &[(f64, f64)]) -> Vec<u8> {
    let mut b = vec![b'G', b'P', 0, 0b0000_0001];
    b.extend(4326i32.to_le_bytes());
    b.push(1);
    b.extend(3u32.to_le_bytes());
    b.extend(1u32.to_le_bytes());
    b.extend((coords.len() as u32).to_le_bytes());
    for (x, y) in coords {
        b.extend(x.to_le_bytes());
        b.extend(y.to_le_bytes());
    }
    b
}

fn square(x: f64, y: f64) -> Vec<u8> {
    gpkg_polygon(&[
        (x, y),
        (x + 1.0, y),
        (x + 1.0, y + 1.0),
        (x, y + 1.0),
        (x, y),
    ])
}

fn build_admin_layer() -> PathBuf {
    let path = std::env::temp_dir().join(format!("mapgen-test-{}.gpkg", std::process::id()));
    let _ = std::fs::remove_file(&path);
    let conn = Connection::open(&path).unwrap();
    conn.execute_batch(
        "CREATE TABLE gpkg_geometry_columns (table_name TEXT, column_name TEXT);
         INSERT INTO gpkg_geometry_columns VALUES ('admin1', 'geom');
         CREATE TABLE admin1 (country TEXT, uid TEXT, name TEXT, iso TEXT, geom BLOB);",
    )
    .unwrap();
    let rows = [
        ("FRA", "fr-11", "Île-de-France", "FR-IDF", square(2.0, 48.0)),
        ("FRA", "fr-05", "Corse", "NA", square(9.0, 42.0)),
        ("BEL", "be-01", "Bruxelles", "BE-BRU", square(4.0, 50.0)),
    ];
    for (g0, g1, name, iso, geom) in rows {
        conn.execute(
            "INSERT INTO admin1 VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params![g0, g1, name, iso, geom],
        )
        .unwrap();
    }
    path
}

#[test]
fn reads_filtered_region_with_iso_ids() {
    let path = build_admin_layer();
    let q = LayerQuery {
        table: Some("admin1".into()),
        id_columns: vec!["iso".into(), "uid".into()],
        name_column: "name".into(),
        filter_column: Some("country".into()),
        class: "subdivision".into(),
    };

    let fr = read_layer(&path, &q, Some("FRA")).unwrap();
    let ids: Vec<&str> = fr.iter().map(|f| f.id.as_str()).collect();
    // The ISO code is used when present; "NA" falls back to the next id column.
    assert_eq!(ids, ["FR-IDF", "fr-05"]);
    assert_eq!(fr[0].name, "Île-de-France");
    assert_eq!(fr[0].geometry.0.len(), 1);

    assert_eq!(list_regions(&path, &q).unwrap(), ["BEL", "FRA"]);
    let _ = std::fs::remove_file(&path);
}
