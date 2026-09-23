//! Tests of the JavaScript-facing API, compiled to wasm32 and run by
//! `wasm-pack test --node` and `wasm-pack test --headless --chrome`.
#![cfg(target_arch = "wasm32")]

use js_sys::{Array, Reflect, JSON};
use mapgen_wasm::{bbox_presets, themes, version, MapGenerator};
use wasm_bindgen::{JsCast, JsValue};

use wasm_bindgen_test::wasm_bindgen_test;

const TWIN: &str = include_str!("../../mapgen-data/tests/fixtures/twin-regions.geojson");
const GOLDEN: &str = include_str!("../../mapgen-data/tests/fixtures/twin-regions.svg");

fn obj<T: JsCast>(json: &str) -> Option<T> {
    Some(JSON::parse(json).expect("valid JSON").unchecked_into())
}

fn get(v: &JsValue, key: &str) -> JsValue {
    Reflect::get(v, &JsValue::from_str(key)).expect("property")
}

fn error_message(e: wasm_bindgen::JsError) -> String {
    let v: JsValue = e.into();
    let err: js_sys::Error = v.dyn_into().expect("a JS Error");
    err.message().into()
}

fn twin() -> MapGenerator {
    let mut g = MapGenerator::new();
    assert_eq!(g.set_subject(TWIN, None).unwrap(), 2);
    g
}

#[wasm_bindgen_test]
fn wasm_output_matches_native_golden_byte_for_byte() {
    let out = twin()
        .render(obj(
            r#"{"width": 400, "title": "Twin regions", "labels": true}"#,
        ))
        .unwrap();
    assert_eq!(get(&out, "svg").as_string().unwrap(), GOLDEN);
}

#[wasm_bindgen_test]
fn output_is_a_plain_object_with_metadata() {
    let out = twin().render(None).unwrap();
    assert_eq!(get(&out, "width").as_f64(), Some(1000.0));
    assert!(get(&out, "height").as_f64().unwrap() > 0.0);
    assert_eq!(get(&out, "projection").as_string().unwrap(), "laea");
    assert_eq!(get(&out, "regions").as_f64(), Some(2.0));
    assert!(Array::is_array(&get(&out, "outsideFrame")));
    assert!(Array::is_array(&get(&out, "center")));
    assert!(get(&out, "html").is_undefined());
}

#[wasm_bindgen_test]
fn html_format_and_css_vars() {
    let out = twin()
        .render(obj(r#"{"format": "html", "title": "T"}"#))
        .unwrap();
    let html = get(&out, "html").as_string().unwrap();
    assert!(html.starts_with("<!doctype html>") && html.contains("Download SVG"));
    assert!(get(&out, "svg")
        .as_string()
        .unwrap()
        .contains("var(--mg-water,#c6ecff)"));
}

#[wasm_bindgen_test]
fn colors_and_themes_apply() {
    let out = twin()
        .render(obj(
            r##"{"theme": "dark", "colors": {"earth": "#abcdef", "background": "none"}}"##,
        ))
        .unwrap();
    let svg = get(&out, "svg").as_string().unwrap();
    assert!(svg.contains(".mg-land{fill:#abcdef;"));
    assert!(svg.contains(".mg-background{fill:none}"));
    assert!(svg.contains(".mg-water{fill:#0d1b2a}"), "dark theme water");
}

#[wasm_bindgen_test]
fn regions_and_filtering() {
    let mut g = MapGenerator::new();
    g.set_subject(TWIN, obj(r#"{"filterProperty": "name"}"#))
        .unwrap();
    assert_eq!(
        g.regions().unwrap(),
        vec!["East & Co".to_string(), "Westland".to_string()]
    );
    let out = g.render(obj(r#"{"region": "Westland"}"#)).unwrap();
    assert_eq!(get(&out, "regions").as_f64(), Some(1.0));
}

#[wasm_bindgen_test]
fn context_layer_can_be_set_and_cleared() {
    let mut g = twin();
    assert_eq!(
        g.set_context(Some(TWIN.to_owned()), obj(r#"{"dataset": "custom"}"#))
            .unwrap(),
        2
    );
    assert_eq!(g.set_context(None, None).unwrap(), 0);
}

#[wasm_bindgen_test]
fn errors_are_js_errors_with_useful_messages() {
    let g = MapGenerator::new();
    assert!(error_message(g.render(None).err().expect("an error")).contains("setSubject"));

    let g = twin();
    let msg = |spec: &str| match g.render(obj(spec)) {
        Ok(_) => panic!("expected an error for {spec}"),
        Err(e) => error_message(e),
    };
    assert!(msg(r#"{"colours": {}}"#).contains("colours"));
    assert!(msg(r#"{"width": "wide"}"#).contains("invalid type"));
    assert!(msg(r#"{"theme": "neon"}"#).contains("wikimedia"));
    assert!(msg(r#"{"colors": {"water": "red;}"}}"#).contains("invalid colour"));
    assert!(msg(r#"{"region": "X"}"#).contains("region column"));

    assert!(msg("[1, 2]").contains("plain object"));

    let mut g = MapGenerator::new();
    assert!(g.set_subject("{nope", None).is_err());
    assert!(
        g.set_subject(TWIN, obj(r#"{"datset": "custom"}"#)).is_err(),
        "typo in layer spec"
    );
    assert!(g.set_subject(TWIN, obj(r#"{"dataset": "gadm"}"#)).is_err());
}

#[wasm_bindgen_test]
fn lookup_tables() {
    let t = themes().unwrap();
    assert_eq!(
        get(&get(&t, "wikimedia"), "water").as_string().unwrap(),
        "#c6ecff"
    );
    let b = bbox_presets().unwrap();
    assert!(Array::is_array(&get(&b, "europe")));
    assert_eq!(version(), env!("CARGO_PKG_VERSION"));
}
