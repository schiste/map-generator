//! Golden-file tests: the pipeline must be byte-for-byte deterministic.
//!
//! Regenerate after an intentional output change with:
//! `UPDATE_GOLDEN=1 cargo test -p mapgen-data --test golden`

use std::path::{Path, PathBuf};

use mapgen_core::{render, MapLayers, RenderOptions};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

fn render_fixture() -> String {
    let subject = mapgen_data::geojson::read_features(
        &fixture("twin-regions.geojson"),
        "id",
        "name",
        "region",
    )
    .unwrap();
    let opts = RenderOptions {
        width: 400,
        title: Some("Twin regions".into()),
        labels: true,
        ..RenderOptions::default()
    };
    render(
        &MapLayers {
            subject,
            ..MapLayers::default()
        },
        &opts,
    )
    .unwrap()
    .svg
}

#[test]
fn output_is_stable_across_runs() {
    assert_eq!(render_fixture(), render_fixture());
}

#[test]
fn matches_golden_svg() {
    let actual = render_fixture();
    let golden = fixture("twin-regions.svg");
    if std::env::var_os("UPDATE_GOLDEN").is_some() {
        std::fs::write(&golden, &actual).unwrap();
    }
    let expected = std::fs::read_to_string(&golden)
        .expect("missing golden file; run with UPDATE_GOLDEN=1 to create it");
    assert_eq!(
        actual, expected,
        "SVG output changed; see module docs to regenerate"
    );
}

#[test]
fn features_are_sorted_and_escaped() {
    let svg = render_fixture();
    let a = svg.find("id=\"XA-01\"").unwrap();
    let b = svg.find("id=\"XA-02\"").unwrap();
    assert!(a < b);
    assert!(svg.contains("data-name=\"East &amp; Co\""));
}

#[test]
fn layers_are_in_paint_order() {
    let svg = render_fixture();
    let pos = |s: &str| svg.find(s).unwrap_or_else(|| panic!("missing {s}"));
    assert!(pos("id=\"background\"") < pos("id=\"water\""));
    assert!(pos("id=\"water\"") < pos("<g id=\"land\">"));
}
