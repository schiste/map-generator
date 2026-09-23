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
fn boundary_version_is_on_the_root_and_in_the_credit() {
    let subject = mapgen_data::geojson::read_features(
        &fixture("twin-regions.geojson"),
        "id",
        "name",
        "region",
    )
    .unwrap();
    let opts = RenderOptions {
        attribution: Some("Census".into()),
        boundary_year: Some("2018".into()),
        source_release: Some("USA-ADM2-52423323".into()),
        ..RenderOptions::default()
    };
    let svg = render(
        &MapLayers {
            subject,
            ..MapLayers::default()
        },
        &opts,
    )
    .unwrap()
    .svg;
    assert!(svg.contains(r#"data-boundary-year="2018" data-source-release="USA-ADM2-52423323">"#));
    assert!(svg.contains("<desc id=\"attribution\">Census; boundaries as of 2018</desc>"));
}

#[test]
fn layers_are_in_paint_order() {
    let svg = render_fixture();
    let pos = |s: &str| svg.find(s).unwrap_or_else(|| panic!("missing {s}"));
    assert!(pos("id=\"background\"") < pos("id=\"water\""));
    assert!(pos("id=\"water\"") < pos("<g id=\"land\">"));
}

#[test]
fn labels_switch_on_system_language() {
    let text = r#"{"type":"FeatureCollection","features":[
      {"type":"Feature","properties":{"id":"XA-01","name":"Westland","name_fr":"Ouestland",
        "name_zht":"西地","name_de":"Ein sehr sehr langer westlicher Landesname"},
       "geometry":{"type":"Polygon","coordinates":[[[9,45],[10,45],[10,46],[9,46],[9,45]]]}},
      {"type":"Feature","properties":{"id":"XA-02","name":"Eastland","name_fr":"Eastland"},
       "geometry":{"type":"Polygon","coordinates":[[[10,45],[11,45],[11,46],[10,46],[10,45]]]}}]}"#;
    let query = mapgen_data::LayerQuery {
        id_columns: vec!["id".into()],
        name_column: "name".into(),
        name_language_column: Some("name_{lang}".into()),
        languages: vec!["fr".into(), "de".into(), "zh-Hant".into()],
        class: "region".into(),
        ..mapgen_data::LayerQuery::default()
    };
    let subject: Vec<_> = mapgen_data::geojson::rows_from_str(text, &query)
        .unwrap()
        .into_iter()
        .map(|(_, f)| f)
        .collect();
    assert_eq!(subject[0].names["zh-Hant"], "西地");
    let opts = RenderOptions {
        width: 300,
        labels: true,
        languages: query.languages.clone(),
        ..RenderOptions::default()
    };
    let layers = MapLayers {
        subject,
        ..MapLayers::default()
    };
    let svg = render(&layers, &opts).unwrap().svg;
    // Westland differs in French and Chinese; the German name doesn't fit,
    // so German viewers get no label rather than one placed for another
    // layout. Eastland is the same everywhere: no switch.
    let westland = svg
        .split("<switch>")
        .nth(1)
        .unwrap()
        .split("</switch>")
        .next()
        .unwrap();
    let lines: Vec<&str> = westland.trim().lines().collect();
    assert_eq!(lines.len(), 4, "{westland}");
    assert!(
        lines[0].starts_with("<text systemLanguage=\"fr\" class=\"mg-label\"")
            && lines[0].ends_with(">Ouestland</text>")
    );
    assert_eq!(lines[1], "<g systemLanguage=\"de\"/>");
    assert!(
        lines[2].starts_with("<text systemLanguage=\"zh-Hant\"")
            && lines[2].ends_with(">西地</text>")
    );
    assert!(
        lines[3].starts_with("<text class=\"mg-label\"") && lines[3].ends_with(">Westland</text>")
    );
    assert_eq!(svg.matches("<switch>").count(), 1);
    assert_eq!(svg.matches(">Eastland</text>").count(), 1);
}

#[test]
fn curved_labels_by_target() {
    // A long, thin diagonal band: too narrow for straight text.
    let text = r#"{"type":"FeatureCollection","features":[
      {"type":"Feature","properties":{"id":"XB","name":"Longland"},
       "geometry":{"type":"Polygon","coordinates":[[[0,0],[0.5,0],[10.5,10],[10,10],[0,0]]]}}]}"#;
    let subject: Vec<_> = mapgen_data::geojson::rows_from_str(
        text,
        &mapgen_data::LayerQuery {
            id_columns: vec!["id".into()],
            name_column: "name".into(),
            ..mapgen_data::LayerQuery::default()
        },
    )
    .unwrap()
    .into_iter()
    .map(|(_, f)| f)
    .collect();
    let layers = MapLayers {
        subject,
        ..MapLayers::default()
    };
    let svg = |target| {
        let opts = RenderOptions {
            width: 400,
            labels: true,
            target,
            ..RenderOptions::default()
        };
        render(&layers, &opts).unwrap().svg
    };
    let commons = svg(mapgen_core::Target::Commons);
    assert!(!commons.contains("textPath") && !commons.contains("dominant-baseline"));
    assert!(commons
        .contains("<g class=\"mg-label\" aria-label=\"Longland\"><text transform=\"translate("));
    assert_eq!(commons.matches(" rotate(").count(), "Longland".len());
    let web = svg(mapgen_core::Target::Web);
    assert!(
        web.contains("<textPath href=\"#label-path-XB\" startOffset=\"50%\">Longland</textPath>")
    );
}
