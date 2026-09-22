//! Reads a tiny GeoPackage built on the fly, so the SQL path is tested
//! without downloading GADM or Natural Earth.

use std::path::PathBuf;

use mapgen_data::{list_regions, read_layer, Source};
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

fn build_gadm_like() -> PathBuf {
    let path = std::env::temp_dir().join(format!("mapgen-test-{}.gpkg", std::process::id()));
    let _ = std::fs::remove_file(&path);
    let conn = Connection::open(&path).unwrap();
    conn.execute_batch(
        "CREATE TABLE gpkg_geometry_columns (table_name TEXT, column_name TEXT);
         INSERT INTO gpkg_geometry_columns VALUES ('ADM_1', 'geom');
         CREATE TABLE ADM_1 (GID_0 TEXT, GID_1 TEXT, NAME_1 TEXT, ISO_1 TEXT, geom BLOB);",
    )
    .unwrap();
    let rows = [
        (
            "FRA",
            "FRA.11_1",
            "Île-de-France",
            "FR-IDF",
            square(2.0, 48.0),
        ),
        ("FRA", "FRA.5_1", "Corse", "NA", square(9.0, 42.0)),
        ("BEL", "BEL.1_1", "Bruxelles", "BE-BRU", square(4.0, 50.0)),
    ];
    for (g0, g1, name, iso, geom) in rows {
        conn.execute(
            "INSERT INTO ADM_1 VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params![g0, g1, name, iso, geom],
        )
        .unwrap();
    }
    path
}

#[test]
fn reads_filtered_region_with_iso_ids() {
    let path = build_gadm_like();
    let q = Source::Gadm { level: 1 }.layer_query();

    let fr = read_layer(&path, &q, Some("FRA")).unwrap();
    let ids: Vec<&str> = fr.iter().map(|f| f.id.as_str()).collect();
    // ISO_1 is used when present; "NA" falls back to GID_1.
    assert_eq!(ids, ["FR-IDF", "FRA.5_1"]);
    assert_eq!(fr[0].name, "Île-de-France");
    assert_eq!(fr[0].geometry.0.len(), 1);

    assert_eq!(list_regions(&path, &q).unwrap(), ["BEL", "FRA"]);
    let _ = std::fs::remove_file(&path);
}
