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
        ..LayerQuery::default()
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

mod writer {
    use mapgen_data::geojson::records_from_str;
    use mapgen_data::gpkg_write::{write_gpkg, WriteOptions};
    use mapgen_data::{read_layer, read_layer_in, LayerQuery, Source};
    use rusqlite::Connection;

    const TWO_COUNTRIES: &str = r#"{"type":"FeatureCollection","features":[
      {"type":"Feature","properties":{"shapeName":"Île-de-France","shapeID":"h1","shapeGroup":"FRA","code":"FR-IDF","parent":"FR-METRO","pop":12.3,"odd key!":"x"},
       "geometry":{"type":"Polygon","coordinates":[[[2,48],[3,48],[3,49],[2,49],[2,48]]]}},
      {"type":"Feature","properties":{"shapeName":"Bruxelles","shapeID":"h2","shapeGroup":"BEL"},
       "geometry":{"type":"MultiPolygon","coordinates":[[[[4,50],[5,50],[5,51],[4,51],[4,50]]]]}}
    ]}"#;

    fn written(name: &str) -> std::path::PathBuf {
        let path =
            std::env::temp_dir().join(format!("mapgen-write-{}-{name}.gpkg", std::process::id()));
        let records = records_from_str(TWO_COUNTRIES).unwrap();
        let opts = WriteOptions {
            table: "FRA-ADM1".into(),
            // "code" twice: an id column also asked for as such is indexed once.
            index_columns: vec!["shapeGroup".into(), "code".into(), "CODE".into()],
        };
        write_gpkg(&path, &records, &opts).unwrap();
        path
    }

    #[test]
    fn round_trips_through_the_geoboundaries_preset() {
        let path = written("roundtrip");
        // The preset has no table name: the file's only layer is used.
        let q = Source::GeoBoundaries.layer_query();
        let fr = read_layer(&path, &q, Some("FRA")).unwrap();
        assert_eq!(fr.len(), 1);
        assert_eq!(fr[0].id, "FR-IDF", "the crosswalk code wins over shapeID");
        assert_eq!(fr[0].parent.as_deref(), Some("FR-METRO"));
        assert_eq!(fr[0].name, "Île-de-France");
        let be = read_layer(&path, &q, Some("BEL")).unwrap();
        assert_eq!(be[0].id, "h2", "falls back to shapeID without a code");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn bbox_queries_use_the_rtree() {
        let path = written("bbox");
        let q = Source::GeoBoundaries.layer_query();
        let near_paris = read_layer_in(&path, &q, None, Some([1.0, 47.0, 3.5, 49.5])).unwrap();
        assert_eq!(
            near_paris.iter().map(|f| f.id.as_str()).collect::<Vec<_>>(),
            ["FR-IDF"]
        );
        let both = read_layer_in(&path, &q, None, Some([0.0, 40.0, 10.0, 60.0])).unwrap();
        assert_eq!(both.len(), 2);

        let conn = Connection::open(&path).unwrap();
        let indexes: Vec<String> = conn
            .prepare("SELECT name FROM sqlite_master WHERE type = 'index' AND name LIKE 'idx_%' ORDER BY name")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(indexes, ["idx_FRA_ADM1_code", "idx_FRA_ADM1_shapeGroup"]);
        let rtree_rows: i64 = conn
            .query_row("SELECT count(*) FROM rtree_FRA_ADM1_geom", [], |r| r.get(0))
            .unwrap();
        assert_eq!(rtree_rows, 2);
        let columns: Vec<String> = conn
            .prepare("SELECT name FROM pragma_table_info('FRA_ADM1')")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert!(columns.contains(&"odd_key_".to_string()), "{columns:?}");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_custom_query_reads_any_column() {
        let path = written("custom");
        let q = LayerQuery {
            table: None,
            id_columns: vec!["shapeID".into()],
            name_column: "shapeName".into(),
            class: "x".into(),
            ..LayerQuery::default()
        };
        assert_eq!(read_layer(&path, &q, None).unwrap().len(), 2);
        let _ = std::fs::remove_file(&path);
    }
}
