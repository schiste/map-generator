//! Golden-file tests: the pipeline must be byte-for-byte deterministic.
//!
//! Regenerate after an intentional output change with:
//! `UPDATE_GOLDEN=1 cargo test -p mapgen-data --test golden`

use std::path::{Path, PathBuf};

use mapgen_core::{render, RenderOptions, SvgOptions};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

fn render_fixture() -> String {
    let features = mapgen_data::geojson::read_features(
        &fixture("twin-regions.geojson"),
        "id",
        "name",
        "subdivision",
    )
    .unwrap();
    let opts = RenderOptions {
        svg: SvgOptions {
            width: 400,
            title: Some("Twin regions".into()),
            ..SvgOptions::default()
        },
        simplify_px: 0.5,
    };
    render(&features, &opts).unwrap()
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
